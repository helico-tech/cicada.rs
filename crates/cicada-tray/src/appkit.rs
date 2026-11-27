//! Main-thread AppKit lifecycle and actions.
#![allow(unsafe_code)]

mod settings;
mod view;

use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use cicada_app::Contexts;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply, NSButton,
    NSControlStateValueOn, NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSMenuDelegate, NSMenuItem,
    NSPopUpButton, NSStatusBar, NSStepper, NSSwitch, NSTextField,
};
use objc2_foundation::{
    NSBundle, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer,
    ns_string,
};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::menu::{Action, MenuControl, MenuInput, Terminate, macos_subtitles, menu};
use crate::settings::SettingsForm;
use crate::{Tray, TrayState};
use settings::SettingsWindow;
use view::MenuView;

pub(super) fn run(tray: Tray) -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("the Cicada menu bar app must run on the main thread");
        return ExitCode::FAILURE;
    };
    let os = NSProcessInfo::processInfo().operatingSystemVersion();
    let subtitles = match macos_subtitles((os.majorVersion, os.minorVersion, os.patchVersion)) {
        Ok(subtitles) => subtitles,
        Err((error, code)) => {
            eprintln!("{error}");
            return code;
        }
    };
    match start(tray, mtm, subtitles) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn start(tray: Tray, mtm: MainThreadMarker, subtitles: bool) -> Result<(), String> {
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
            autoreleasepool(|_| run_app(mtm, tray, interrupted, subtitles))
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

fn app_name() -> String {
    let bundle = NSBundle::mainBundle();
    for key in [ns_string!("CFBundleDisplayName"), ns_string!("CFBundleName")] {
        if let Some(name) = bundle.objectForInfoDictionaryKey(key).and_then(|value| value.downcast::<NSString>().ok()) {
            return name.to_string();
        }
    }
    NSProcessInfo::processInfo().processName().to_string()
}

fn run_app(mtm: MainThreadMarker, tray: Tray, interrupted: Arc<AtomicBool>, subtitles: bool) -> Result<(), String> {
    let app = NSApplication::sharedApplication(mtm);
    let accessory = NSApplicationActivationPolicy::Accessory;
    if app.activationPolicy() != accessory && !app.setActivationPolicy(accessory) {
        return Err("cannot run the Cicada tray as an accessory application".into());
    }

    let title = app_name();
    let bar = NSStatusBar::systemStatusBar();
    let (status, view) = MenuView::new(mtm, &bar, &title, subtitles).ok_or("cannot create the Cicada status item")?;
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
            settings: RefCell::default(),
            menu_open: Cell::new(false),
            last_clock: Cell::new(Instant::now()),
        },
    );

    target.ivars().view.bind(&target);
    app.setMainMenu(Some(&settings::edit_menu(mtm)));
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
    /// The Settings window, made at the first Settings….
    settings: RefCell<Option<SettingsWindow>>,
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
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, _: &NSMenu) {
            self.reload_contexts();
            self.render();
        }

        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _: &NSMenu) {
            self.ivars().menu_open.set(true);
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

        // SAFETY: AppKit passes the NSSwitch or its containing NSMenuItem as the sender.
        #[unsafe(method(trayToggle:))]
        fn toggle(&self, sender: &AnyObject) {
            let c = self.ivars();
            let state = self.state();
            let on = sender.downcast_ref::<NSSwitch>().map_or_else(
                || matches!(state, TrayState::Offline | TrayState::Failed { .. }),
                |switch| switch.state() == NSControlStateValueOn,
            );
            let contexts = c.contexts.borrow();
            let current = contexts.current().filter(|name| contexts.names().iter().any(|context| context == name));
            let action = c.control.borrow().toggle(&state, on, current);
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

        // SAFETY: Only the Stop Now NSMenuItem is wired to this action.
        #[unsafe(method(trayStopNow:))]
        fn stop_now(&self, _: &NSMenuItem) {
            self.ivars().tray.stop_now();
            self.render();
        }

        // SAFETY: Only the Quit NSMenuItem is wired to this action.
        #[unsafe(method(trayQuit:))]
        fn quit(&self, _: &NSMenuItem) {
            self.begin_quit();
        }

        // SAFETY: Only the Settings… NSMenuItem is wired to this action.
        #[unsafe(method(traySettings:))]
        fn open_settings(&self, _: &NSMenuItem) {
            let c = self.ivars();
            if c.settings.borrow().is_none() {
                let registered = c.tray.control.registered_types.lock().expect("tray registered types").clone();
                c.settings.replace(Some(SettingsWindow::new(self, &c.title, &registered)));
            }
            if let Some(window) = c.settings.borrow().as_ref() {
                window.show(|| self.settings_form());
            }
        }

        // SAFETY: Only the Settings window's context NSPopUpButton is wired to this action.
        #[unsafe(method(settingsContext:))]
        fn settings_context(&self, _: &NSPopUpButton) {
            if let Some(window) = self.ivars().settings.borrow().as_ref() {
                window.picked_context();
            }
        }

        // SAFETY: Only the Settings window's NSStepper is wired to this action.
        #[unsafe(method(settingsStep:))]
        fn settings_step(&self, _: &NSStepper) {
            if let Some(window) = self.ivars().settings.borrow().as_ref() {
                window.stepped();
            }
        }

        // SAFETY: Only the Settings window's concurrency NSTextField is wired to this action.
        #[unsafe(method(settingsConcurrency:))]
        fn settings_concurrency(&self, _: &NSTextField) {
            if let Some(window) = self.ivars().settings.borrow().as_ref() {
                window.typed_concurrency();
            }
        }

        // SAFETY: Only the Settings window's Revert NSButton is wired to this action.
        #[unsafe(method(settingsRevert:))]
        fn settings_revert(&self, _: &NSButton) {
            if let Some(window) = self.ivars().settings.borrow().as_ref() {
                window.revert(|| self.settings_form());
            }
        }

        // SAFETY: Only the Settings window's Apply and Reconnect NSButton is wired to this action.
        #[unsafe(method(settingsApply:))]
        fn settings_apply(&self, _: &NSButton) {
            self.apply_settings();
        }

        // SAFETY: Only the Add sheet's Add NSButton is wired to this action.
        #[unsafe(method(settingsAddOk:))]
        fn settings_add_ok(&self, _: &NSButton) {
            if let Some(window) = self.ivars().settings.borrow().as_ref() {
                window.add();
            }
        }

        // SAFETY: Only the Add sheet's Cancel NSButton is wired to this action.
        #[unsafe(method(settingsAddCancel:))]
        fn settings_add_cancel(&self, _: &NSButton) {
            if let Some(window) = self.ivars().settings.borrow().as_ref() {
                window.cancel_add();
            }
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
        let (state, worker_types, ready) = {
            let connection = c.tray.control.connection.lock().expect("tray connection");
            let state = c.state.borrow_mut().borrow_and_update().clone();
            let ready = connection.as_ref().and_then(|connection| connection.ready.as_ref());
            let worker_types = ready.map(|ready| ready.types.clone()).unwrap_or_default();
            (state, worker_types, ready.is_some())
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
            ready,
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

    /// The Settings form from the files on disk, showing the connected context, else the current one.
    fn settings_form(&self) -> Result<SettingsForm, String> {
        let c = self.ivars();
        let registered = c.tray.control.registered_types.lock().expect("tray registered types").clone();
        let state = self.state();
        SettingsForm::load(&c.tray.environment(), &registered, state.connection())
    }

    /// Apply and Reconnect: saves the form and, while the tray holds a connection, reconnects to the applied context.
    fn apply_settings(&self) {
        let c = self.ivars();
        let settings = c.settings.borrow();
        let Some(window) = settings.as_ref() else { return };
        let Some(name) = window.apply() else { return };
        window.refill();
        let state = self.state();
        let action = c.control.borrow_mut().apply(&state, name);
        let reconnecting = !c.control.borrow().quitting() && state.connection().is_some();
        window.status(if reconnecting { "Saved. Reconnecting…" } else { "Saved." });
        drop(settings);
        self.act(action);
        self.reload_contexts();
        self.render();
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
