//! Main-thread AppKit lifecycle and actions.
#![allow(unsafe_code)]

mod view;

use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use cicada_app::Contexts;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply,
    NSControlStateValueOn, NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar,
    NSSwitch,
};
use objc2_foundation::{
    NSBundle, NSObject, NSObjectProtocol, NSOperatingSystemVersion, NSPoint, NSProcessInfo, NSRunLoop,
    NSRunLoopCommonModes, NSString, NSTimer, ns_string,
};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::menu::{Action, MenuControl, MenuInput, Terminate, menu};
use crate::{Tray, TrayState};
use view::MenuView;

pub(super) fn run(tray: Tray) -> ExitCode {
    match start(tray) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn start(tray: Tray) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("the Cicada menu bar app must run on the main thread")?;
    if !at_least(0) {
        return Err("the Cicada menu bar app requires macOS 14 or later".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start the tray's Tokio runtime: {error}"))?;
    let factory = tray.factory.clone();
    let initial_app = runtime.block_on(runtime.spawn_blocking(move || factory()));
    let handle = runtime.enter();
    let result = match initial_app {
        Ok(app) => {
            tray.control.set_registered_types(&app);
            drop(app);
            let interrupted = Arc::new(AtomicBool::new(false));
            runtime.spawn({
                let interrupted = interrupted.clone();
                async move {
                    match tokio::signal::ctrl_c().await {
                        Ok(()) => interrupted.store(true, Ordering::Relaxed),
                        Err(error) => eprintln!("cannot watch Ctrl-C: {error}"),
                    }
                }
            });
            autoreleasepool(|_| run_app(mtm, tray, interrupted))
        }
        Err(error) => {
            drop(tray);
            Err(format!("cannot build the tray's App: {error}"))
        }
    };
    drop(handle);
    runtime.shutdown_timeout(Duration::from_secs(5));
    result
}

fn at_least(minor: isize) -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 14,
        minorVersion: minor,
        patchVersion: 0,
    })
}

fn app_name() -> String {
    let bundle = NSBundle::mainBundle();
    for key in [ns_string!("CFBundleDisplayName"), ns_string!("CFBundleName")] {
        if let Some(name) = bundle.objectForInfoDictionaryKey(key).and_then(|value| value.downcast::<NSString>().ok()) {
            return name.to_string();
        }
    }
    NSProcessInfo::processInfo().processName().to_string()
}

fn run_app(mtm: MainThreadMarker, tray: Tray, interrupted: Arc<AtomicBool>) -> Result<(), String> {
    let app = NSApplication::sharedApplication(mtm);
    if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
        return Err("cannot run the Cicada tray as an accessory application".into());
    }

    let title = app_name();
    let bar = NSStatusBar::systemStatusBar();
    let (status, view) = MenuView::new(mtm, &bar, &title).ok_or("cannot create the Cicada status item")?;
    let state = tray.state();
    let turns = tray.turn_changes();
    let target = Target::new(
        mtm,
        Controller {
            tray,
            title,
            view,
            interrupted,
            state: RefCell::new(state),
            turns: RefCell::new(turns),
            contexts: RefCell::default(),
            control: RefCell::default(),
            menu_open: Cell::new(false),
            last_clock: Cell::new(Instant::now()),
        },
    );

    target.ivars().view.bind(&target);
    target.reload_contexts();
    target.render();
    app.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    // SAFETY: The target implements trayTick: with NSTimer's one-argument signature, and is retained by the timer.
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(0.25, &target, sel!(trayTick:), None, true)
    };
    // SAFETY: This is the main thread's run loop and the timer is also invalidated on this thread.
    unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };

    app.run();

    timer.invalidate();
    app.setDelegate(None);
    target.ivars().view.root.setDelegate(None);
    status.setMenu(None);
    bar.removeStatusItem(&status);
    Ok(())
}

struct Controller {
    tray: Tray,
    title: String,
    view: MenuView,
    interrupted: Arc<AtomicBool>,
    state: RefCell<watch::Receiver<TrayState>>,
    turns: RefCell<watch::Receiver<u64>>,
    contexts: RefCell<Contexts>,
    control: RefCell<MenuControl>,
    menu_open: Cell<bool>,
    last_clock: Cell<Instant>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Target does not implement Drop.
    #[unsafe(super = NSObject)]
    #[name = "CicadaTrayMenuTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Controller]
    struct Target;

    // SAFETY: NSObjectProtocol has no implementation requirements.
    unsafe impl NSObjectProtocol for Target {}

    // SAFETY: These callbacks have the signatures required by NSMenuDelegate.
    unsafe impl NSMenuDelegate for Target {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _: &NSMenu) {
            self.ivars().menu_open.set(true);
            self.reload_contexts();
            self.render();
        }

        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _: &NSMenu) {
            self.ivars().menu_open.set(false);
        }
    }

    // SAFETY: The callback has the signature required by NSApplicationDelegate.
    unsafe impl NSApplicationDelegate for Target {
        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(&self, _: &NSApplication) -> NSApplicationTerminateReply {
            let action = self.ivars().control.borrow_mut().terminate(&self.state());
            match action {
                Terminate::Now => NSApplicationTerminateReply::TerminateNow,
                Terminate::Later(action) => {
                    self.act(action);
                    NSApplicationTerminateReply::TerminateLater
                }
            }
        }
    }

    impl Target {
        // SAFETY: NSTimer sends trayTick: with the timer as the only argument.
        #[unsafe(method(trayTick:))]
        fn tick(&self, timer: &NSTimer) {
            let c = self.ivars();
            if c.interrupted.swap(false, Ordering::Relaxed) {
                self.begin_quit();
            }
            let action = c.control.borrow_mut().tick(&self.state());
            match action {
                Some(Action::StopApp | Action::ReplyToTerminate) => {
                    timer.invalidate();
                    self.finish_quit(action == Some(Action::ReplyToTerminate));
                    return;
                }
                _ => self.act(action),
            }

            let changed = c.state.borrow().has_changed().unwrap_or(false);
            let turns_changed = c.turns.borrow().has_changed().unwrap_or(false);
            let clock_tick = c.last_clock.get().elapsed() >= Duration::from_secs(1);
            if changed || (c.menu_open.get() && (turns_changed || clock_tick)) {
                self.render();
            }
        }

        // SAFETY: Only the header's NSSwitch is wired to this action.
        #[unsafe(method(trayToggle:))]
        fn toggle(&self, sender: &NSSwitch) {
            let c = self.ivars();
            let contexts = c.contexts.borrow();
            let current = contexts.current().filter(|name| contexts.names().iter().any(|context| context == name));
            let action = c.control.borrow().toggle(&self.state(), sender.state() == NSControlStateValueOn, current);
            drop(contexts);
            self.act(action);
            self.render();
        }

        // SAFETY: Only the context NSMenuItems are wired to this action.
        #[unsafe(method(trayPickContext:))]
        fn pick_context(&self, sender: &NSMenuItem) {
            let action = self.ivars().control.borrow_mut().pick(&self.state(), sender.title().to_string());
            self.act(action);
            self.render();
        }

        // SAFETY: Only the Quit NSMenuItem is wired to this action.
        #[unsafe(method(trayQuit:))]
        fn quit(&self, _: &NSMenuItem) {
            self.begin_quit();
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, controller: Controller) -> Retained<Self> {
        let target = Self::alloc(mtm).set_ivars(controller);
        // SAFETY: The ivars are set, and NSObject's init has no additional requirements.
        unsafe { msg_send![super(target), init] }
    }

    fn state(&self) -> TrayState {
        self.ivars().state.borrow().borrow().clone()
    }

    fn reload_contexts(&self) {
        let c = self.ivars();
        let contexts = Contexts::load(&c.tray.environment()).unwrap_or_else(|error| {
            eprintln!("cannot load tray contexts: {error}");
            Contexts::default()
        });
        c.contexts.replace(contexts);
    }

    fn render(&self) {
        let c = self.ivars();
        let (state, worker_types) = {
            let connection = c.tray.control.connection.lock().expect("tray connection");
            let state = c.state.borrow_mut().borrow_and_update().clone();
            let worker_types = connection
                .as_ref()
                .and_then(|connection| connection.ready.as_ref())
                .map(|ready| ready.types.clone())
                .unwrap_or_default();
            (state, worker_types)
        };
        c.turns.borrow_mut().borrow_and_update();
        let registered_types = c.tray.control.registered_types.lock().expect("tray registered types").clone();
        let contexts = c.contexts.borrow();
        let now = Instant::now();
        let model = menu(&MenuInput {
            title: &c.title,
            state: &state,
            turns: &c.tray.turns(),
            registered_types: &registered_types,
            worker_types: &worker_types,
            contexts: contexts.names(),
            current: contexts.current(),
            now: SystemTime::now().into(),
            monotonic_now: now,
            quitting: c.control.borrow().quitting(),
        });
        drop(contexts);
        c.view.render(self, model);
        c.last_clock.set(now);
    }

    fn act(&self, action: Option<Action>) {
        let tray = &self.ivars().tray;
        match action {
            Some(Action::Connect(name)) => tray.connect(name),
            Some(Action::Disconnect) => tray.disconnect(),
            Some(Action::StopApp | Action::ReplyToTerminate) | None => {}
        }
    }

    fn begin_quit(&self) {
        let action = self.ivars().control.borrow_mut().quit(&self.state());
        self.act(action);
        self.render();
    }

    fn finish_quit(&self, system: bool) {
        let app = NSApplication::sharedApplication(self.mtm());
        if self.ivars().menu_open.get() {
            self.ivars().view.root.cancelTracking();
        }
        if system {
            app.replyToApplicationShouldTerminate(true);
        } else {
            app.stop(None);
            // AppKit checks its stop flag after dispatching an event.
            if let Some(event) =
                NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                    NSEventType::ApplicationDefined,
                    NSPoint::ZERO,
                    NSEventModifierFlags::empty(),
                    0.0,
                    0,
                    None,
                    0,
                    0,
                    0,
                )
            {
                app.postEvent_atStart(&event, true);
            }
        }
    }
}
