//! Main-thread AppKit lifecycle: the window with its sidebar, toolbar and content, the main menu, and their actions.
#![allow(unsafe_code)]

mod content;

use std::cell::{Cell, OnceCell, RefCell};
use std::process::ExitCode;
use std::rc::{Rc, Weak};
use std::time::Duration;

use cicada_app::Env;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSControl, NSControlStateValueOn, NSControlTextEditingDelegate,
    NSEvent, NSEventModifierFlags, NSEventType, NSImage, NSImageView, NSMenu, NSMenuItem, NSPopUpButton, NSScrollView,
    NSSplitViewController, NSSplitViewItem, NSTableCellView, NSTableColumn, NSTableColumnResizingOptions, NSTableView,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextField,
    NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem,
    NSToolbarItemIdentifier, NSToolbarSidebarTrackingSeparatorItemIdentifier, NSView, NSViewController, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWindowToolbarStyle,
};
use objc2_foundation::{
    NSArray, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer, ns_string,
};
use tokio::{runtime::Handle, sync::watch};

use crate::lifecycle::WindowSlot;
use crate::model::{Limits, Navigation, Section, Session, View, check_macos};
use content::FlippedView;

/// The rows of the sidebar: the Overview, then the items under Browse.
#[derive(Clone, Copy)]
enum Row {
    Item(Section),
    Group(&'static str),
}

/// The number of sidebar rows: the Overview, the Browse heading and its items.
const ROWS: usize = Section::BROWSE.len() + 2;

fn row_at(row: NSInteger) -> Option<Row> {
    match usize::try_from(row).ok()? {
        0 => Some(Row::Item(Section::Overview)),
        1 => Some(Row::Group("Browse")),
        n => Section::BROWSE.get(n - 2).copied().map(Row::Item),
    }
}

const BACK: &str = "inspector.back";
const FORWARD: &str = "inspector.forward";
const CONTEXT: &str = "inspector.context";
const REFRESH: &str = "inspector.refresh";

/// A main-thread inspector in an existing macOS 14 or later AppKit application.
///
/// The host owns the application and Tokio runtime. Keep the controller alive for as long as the window may be open.
pub struct Embedded {
    mtm: MainThreadMarker,
    env: Env,
    handle: Handle,
    windows: Rc<Windows>,
}

type Windows = WindowSlot<Retained<Target>>;

impl Embedded {
    /// Uses the host's main thread, environment and Tokio runtime.
    pub fn new(mtm: MainThreadMarker, env: Env, handle: Handle) -> Self {
        Self { mtm, env, handle, windows: Rc::default() }
    }

    /// Opens the inspector on `context` or resolves the target as `cicada` does when it is `None`.
    /// If already open, brings the window forward and keeps its current context.
    pub fn show(&self, context: Option<&str>) {
        let target = self.windows.show(|| {
            let session = Session::new(self.env.clone(), self.handle.clone(), Limits::DEFAULT);
            let target = Target::new(self.mtm, session, Some(Rc::downgrade(&self.windows)));
            target.start(context);
            target
        });
        target.show();
    }

    /// Closes the window and requests cancellation of any refresh. A later show starts a fresh session.
    pub fn close(&self) {
        if let Some(target) = self.windows.take() {
            target.close();
        }
    }
}

impl Drop for Embedded {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn run(env: Env) -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("the Cicada inspector must run on the main thread");
        return ExitCode::FAILURE;
    };
    let os = NSProcessInfo::processInfo().operatingSystemVersion();
    if let Err(error) = check_macos((os.majorVersion, os.minorVersion, os.patchVersion)) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cannot start the inspector's Tokio runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let session = Session::new(env, runtime.handle().clone(), Limits::DEFAULT);
    // The session, and with it every pending refresh, is dropped before the runtime shuts down.
    autoreleasepool(|_| run_app(mtm, session));
    runtime.shutdown_timeout(Duration::from_secs(1));
    ExitCode::SUCCESS
}

fn run_app(mtm: MainThreadMarker, session: Session) {
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let target = Target::new(mtm, session, None);
    app.setMainMenu(Some(&main_menu(mtm, &target)));
    app.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    target.start(None);
    target.show();

    app.run();

    target.close();
    app.setDelegate(None);
    app.setMainMenu(None);
}

/// The names, the selected context and the overrides that the context pop-up shows.
type ContextsShown = (Vec<String>, Option<String>, Vec<&'static str>);

struct Controller {
    session: Session,
    /// Only embedded windows return ownership to a host when closed.
    owner: Option<Weak<Windows>>,
    view: RefCell<watch::Receiver<View>>,
    navigation: RefCell<Navigation>,
    window: OnceCell<Window>,
    back: RefCell<Option<Retained<NSToolbarItem>>>,
    forward: RefCell<Option<Retained<NSToolbarItem>>>,
    refresh: RefCell<Option<Retained<NSToolbarItem>>>,
    /// What the context pop-up shows, so that it is rebuilt only when that changes.
    contexts_shown: RefCell<Option<ContextsShown>>,
    /// True while the sidebar's selection is set from the navigation, so that its change is not taken as a pick.
    syncing: Cell<bool>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    finished: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Target does not implement Drop.
    #[unsafe(super = NSObject)]
    #[name = "CicadaInspectorTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Controller]
    struct Target;

    // SAFETY: NSObjectProtocol has no implementation requirements.
    unsafe impl NSObjectProtocol for Target {}

    // SAFETY: The callback has the signature required by NSApplicationDelegate.
    unsafe impl NSApplicationDelegate for Target {
        /// Quit: the timer stops and pending refreshes are dropped before AppKit ends the process.
        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(&self, _: &NSApplication) -> NSApplicationTerminateReply {
            self.close();
            NSApplicationTerminateReply::TerminateNow
        }
    }

    // SAFETY: These callbacks have the signatures required by NSWindowDelegate.
    unsafe impl NSWindowDelegate for Target {
        /// A native close releases the embedded window, or ends the standalone app.
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _: &NSNotification) {
            let target = self.retain();
            if target.teardown() {
                match &target.ivars().owner {
                    Some(owner) => {
                        if let Some(windows) = owner.upgrade() {
                            windows.closed(|window| std::ptr::eq(&**window, &*target));
                        }
                    }
                    None => stop_app(target.mtm()),
                }
            }
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _: &NSNotification) {
            if let Some(window) = self.ivars().window.get() {
                let view = self.ivars().view.borrow().borrow().clone();
                let section = self.ivars().navigation.borrow().current();
                content::render(&window.document, &window.content, section, &view);
            }
        }
    }

    // SAFETY: NSControlTextEditingDelegate has only optional methods.
    unsafe impl NSControlTextEditingDelegate for Target {}

    // SAFETY: The callback has the signature required by NSTableViewDataSource.
    unsafe impl NSTableViewDataSource for Target {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _: &NSTableView) -> NSInteger {
            ROWS as NSInteger
        }
    }

    // SAFETY: These callbacks have the signatures required by NSTableViewDelegate.
    unsafe impl NSTableViewDelegate for Target {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(&self, _: &NSTableView, _: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
            row_at(row).map(|row| sidebar_cell(self.mtm(), row))
        }

        #[unsafe(method(tableView:isGroupRow:))]
        fn is_group_row(&self, _: &NSTableView, row: NSInteger) -> bool {
            matches!(row_at(row), Some(Row::Group(_)))
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, _: &NSTableView, row: NSInteger) -> bool {
            matches!(row_at(row), Some(Row::Item(_)))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, _: &NSNotification) {
            let c = self.ivars();
            if c.syncing.get() {
                return;
            }
            let row = self.window().sidebar.selectedRow();
            if let Some(Row::Item(section)) = row_at(row) {
                c.navigation.borrow_mut().select(section);
                self.render();
            }
        }
    }

    // SAFETY: These callbacks have the signatures required by NSToolbarDelegate.
    unsafe impl NSToolbarDelegate for Target {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(
            &self,
            _: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.toolbar_item_for(identifier)
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            toolbar_identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_items(&self, _: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            toolbar_identifiers()
        }
    }

    impl Target {
        // SAFETY: NSTimer sends inspectorTick: with the timer as the only argument.
        #[unsafe(method(inspectorTick:))]
        fn tick(&self, _: &NSTimer) {
            if self.ivars().view.borrow().has_changed().unwrap_or(false) {
                self.render();
            }
        }

        // SAFETY: The Back toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorBack:))]
        fn back(&self, _: Option<&AnyObject>) {
            if self.ivars().navigation.borrow_mut().back() {
                self.render();
            }
        }

        // SAFETY: The Forward toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorForward:))]
        fn forward(&self, _: Option<&AnyObject>) {
            if self.ivars().navigation.borrow_mut().forward() {
                self.render();
            }
        }

        // SAFETY: The Refresh toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorRefresh:))]
        fn refresh(&self, _: Option<&AnyObject>) {
            if !self.ivars().finished.get() {
                self.ivars().session.refresh();
                self.render();
            }
        }

        // SAFETY: Only the context NSPopUpButton is wired to this action.
        #[unsafe(method(inspectorContext:))]
        fn pick_context(&self, sender: &NSPopUpButton) {
            if self.ivars().finished.get() {
                return;
            }
            let Some(item) = sender.selectedItem() else { return };
            let name = item.title().to_string();
            self.ivars().session.use_context(Some(name));
            self.render();
        }

        /// Back, Forward and Refresh in the menu are enabled as their toolbar items are.
        // SAFETY: The signature is NSMenuItemValidation's validateMenuItem:.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let action = item.action();
            let navigation = self.ivars().navigation.borrow();
            match action {
                Some(action) if action == sel!(inspectorBack:) => navigation.can_back(),
                Some(action) if action == sel!(inspectorForward:) => navigation.can_forward(),
                Some(action) if action == sel!(inspectorRefresh:) => self.can_refresh(),
                _ => true,
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, session: Session, owner: Option<Weak<Windows>>) -> Retained<Self> {
        let view = session.view();
        let allocated = Self::alloc(mtm).set_ivars(Controller {
            session,
            owner,
            view: RefCell::new(view),
            navigation: RefCell::default(),
            window: OnceCell::new(),
            back: RefCell::default(),
            forward: RefCell::default(),
            refresh: RefCell::default(),
            contexts_shown: RefCell::default(),
            syncing: Cell::new(false),
            timer: RefCell::default(),
            finished: Cell::new(false),
        });
        // SAFETY: The ivars are set, and NSObject's init has no additional requirements.
        let target: Retained<Self> = unsafe { msg_send![super(allocated), init] };
        let _ = target.ivars().window.set(Window::new(&target));
        let window = target.window();
        window.window.setToolbar(Some(&window.toolbar));
        target
    }

    fn start(&self, context: Option<&str>) {
        self.ivars().session.use_context(context.map(str::to_owned));
        self.render();
        self.window().window.center();
        // SAFETY: inspectorTick: has NSTimer's one-argument signature, and the timer retains the target until teardown.
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                0.25,
                self,
                sel!(inspectorTick:),
                None,
                true,
            )
        };
        // SAFETY: This is the main thread's run loop and teardown also invalidates the timer on this thread.
        unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        self.ivars().timer.replace(Some(timer));
    }

    fn show(&self) {
        NSApplication::sharedApplication(self.mtm()).activate();
        if !self.ivars().finished.get() {
            self.window().window.makeKeyAndOrderFront(None);
        }
    }

    fn window(&self) -> &Window {
        self.ivars().window.get().expect("the window is made before the app runs")
    }

    fn can_refresh(&self) -> bool {
        self.ivars().view.borrow().borrow().target_error.is_none()
    }

    /// Shows the latest view: the title, the toolbar, the sidebar's selection and the content.
    fn render(&self) {
        let c = self.ivars();
        if c.finished.get() {
            return;
        }
        let Some(window) = c.window.get() else { return };
        let view = c.view.borrow_mut().borrow_and_update().clone();
        let (section, can_back, can_forward) = {
            let navigation = c.navigation.borrow();
            (navigation.current(), navigation.can_back(), navigation.can_forward())
        };
        window.window.setTitle(&NSString::from_str(section.title()));
        window.window.setSubtitle(&NSString::from_str(&subtitle(&view)));
        for (item, enabled) in
            [(&c.back, can_back), (&c.forward, can_forward), (&c.refresh, view.target_error.is_none())]
        {
            let item = item.borrow().clone();
            if let Some(item) = item {
                item.setEnabled(enabled);
            }
        }
        self.fill_contexts(&view);
        let row = (0..ROWS).find(|&row| matches!(row_at(row as NSInteger), Some(Row::Item(s)) if s == section));
        if let Some(row) = row
            && window.sidebar.selectedRow() != row as NSInteger
        {
            c.syncing.set(true);
            window.sidebar.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false);
            c.syncing.set(false);
        }
        content::render(&window.document, &window.content, section, &view);
    }

    /// The context pop-up: the names of the config file, the context in use picked; then, when the environment wins
    /// over it, a line that says which variables do.
    fn fill_contexts(&self, view: &View) {
        let c = self.ivars();
        let shown =
            (view.contexts.names().to_vec(), view.context.clone().or(view.picked.clone()), view.overrides.clone());
        if c.contexts_shown.borrow().as_ref() == Some(&shown) {
            return;
        }
        let popup = &self.window().contexts;
        let mtm = self.mtm();
        popup.removeAllItems();
        let menu = popup.menu().expect("a pop-up button has a menu");
        let (names, selected, overrides) = &shown;
        for name in names {
            let item = menu_item(mtm, name, None, "");
            menu.addItem(&item);
            if Some(name) == selected.as_ref() {
                item.setState(NSControlStateValueOn);
                popup.selectItem(Some(&item));
            }
        }
        if names.is_empty() {
            let item = menu_item(mtm, "No contexts", None, "");
            item.setEnabled(false);
            menu.addItem(&item);
        }
        if !overrides.is_empty() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            let item = menu_item(mtm, &override_text(overrides), None, "");
            item.setEnabled(false);
            menu.addItem(&item);
        }
        if selected.as_ref().is_none_or(|selected| !names.contains(selected)) {
            popup.selectItem(None);
        }
        popup.setAccessibilityLabel(Some(ns_string!("Context")));
        c.contexts_shown.replace(Some(shown));
    }

    fn toolbar_item_for(&self, identifier: &NSToolbarItemIdentifier) -> Option<Retained<NSToolbarItem>> {
        let mtm = self.mtm();
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), identifier);
        let c = self.ivars();
        let button = |symbol: &str, label: &str, action: Sel| {
            item.setLabel(&NSString::from_str(label));
            item.setToolTip(Some(&NSString::from_str(label)));
            item.setImage(symbol_image(symbol, label).as_deref());
            item.setBordered(true);
            item.setAutovalidates(false);
            // SAFETY: The target implements the action with the item as the sender, and outlives the toolbar.
            unsafe {
                item.setTarget(Some(self));
                item.setAction(Some(action));
            }
        };
        match identifier.to_string().as_str() {
            BACK => {
                button("chevron.left", "Back", sel!(inspectorBack:));
                item.setNavigational(true);
                c.back.replace(Some(item.clone()));
            }
            FORWARD => {
                button("chevron.right", "Forward", sel!(inspectorForward:));
                item.setNavigational(true);
                c.forward.replace(Some(item.clone()));
            }
            REFRESH => {
                button("arrow.clockwise", "Refresh", sel!(inspectorRefresh:));
                c.refresh.replace(Some(item.clone()));
            }
            CONTEXT => {
                item.setLabel(ns_string!("Context"));
                item.setToolTip(Some(ns_string!("The context of cicada config")));
                item.setView(Some(&self.window().contexts));
            }
            _ => return None,
        }
        Some(item)
    }

    fn close(&self) {
        if self.teardown() {
            self.window().window.close();
        }
    }

    /// Ends this window's work once, on a native close, a host close or Quit.
    fn teardown(&self) -> bool {
        let c = self.ivars();
        if c.finished.replace(true) {
            return false;
        }
        let timer = c.timer.take();
        if let Some(timer) = timer {
            timer.invalidate();
        }
        c.session.stop();
        let window = self.window();
        window.window.setDelegate(None);
        window.toolbar.setDelegate(None);
        // SAFETY: Clears the table's references to the target while it is still alive.
        unsafe {
            window.sidebar.setDataSource(None);
            window.sidebar.setDelegate(None);
        }
        true
    }
}

/// The window's subtitle: the context in use, and the variables that win over it.
fn subtitle(view: &View) -> String {
    let context = view.context.as_deref().or(view.picked.as_deref()).unwrap_or("No context");
    if view.overrides.is_empty() {
        context.to_owned()
    } else {
        format!("{context} · {}", override_text(&view.overrides))
    }
}

/// `CICADA_SERVER and CICADA_TOKEN override the context`, for the variables set.
pub(super) fn override_text(overrides: &[&str]) -> String {
    let verb = if overrides.len() == 1 { "overrides" } else { "override" };
    format!("{} {verb} the context", overrides.join(" and "))
}

/// The parts of the window that the target fills.
struct Window {
    window: Retained<NSWindow>,
    sidebar: Retained<NSTableView>,
    content: Retained<NSScrollView>,
    document: Retained<FlippedView>,
    contexts: Retained<NSPopUpButton>,
    /// Attached once the target holds the window, as its items need the pop-up.
    toolbar: Retained<NSToolbar>,
    // Kept for the life of the window; AppKit holds them too.
    _split: Retained<NSSplitViewController>,
}

impl Window {
    fn new(target: &Target) -> Self {
        let mtm = target.mtm();
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::FullSizeContentView;
        let rect = NSRect::new(NSPoint::ZERO, NSSize::new(980.0, 640.0));
        // SAFETY: NSWindow's designated initializer, with a valid style mask and backing store.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: The controller holds the window in a Retained, so closing must not release it.
        unsafe { window.setReleasedWhenClosed(false) };

        let sidebar = NSTableView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("item"));
        column.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
        sidebar.addTableColumn(&column);
        sidebar.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::FirstColumnOnlyAutoresizingStyle);
        sidebar.setHeaderView(None);
        sidebar.setStyle(NSTableViewStyle::SourceList);
        sidebar.setFloatsGroupRows(false);
        sidebar.setAllowsEmptySelection(false);
        sidebar.setRowHeight(28.0);
        sidebar.setAccessibilityLabel(Some(ns_string!("Sidebar")));
        // SAFETY: The target implements both protocols and clears them during teardown.
        unsafe {
            sidebar.setDataSource(Some(ProtocolObject::from_ref(target)));
            sidebar.setDelegate(Some(ProtocolObject::from_ref(target)));
        }
        let sidebar_scroll = NSScrollView::new(mtm);
        sidebar_scroll.setDocumentView(Some(&sidebar));
        sidebar_scroll.setDrawsBackground(false);
        sidebar_scroll.setHasVerticalScroller(true);
        sidebar_scroll.setAutohidesScrollers(true);
        let sidebar_controller = NSViewController::new(mtm);
        sidebar_controller.setView(&sidebar_scroll);
        let sidebar_item = NSSplitViewItem::sidebarWithViewController(&sidebar_controller);
        sidebar_item.setMinimumThickness(180.0);
        sidebar_item.setMaximumThickness(280.0);
        sidebar_item.setCanCollapse(false);

        let content = NSScrollView::new(mtm);
        content.setHasVerticalScroller(true);
        content.setAutohidesScrollers(true);
        let document: Retained<FlippedView> = {
            let rect = NSRect::new(NSPoint::ZERO, NSSize::new(760.0, 600.0));
            // SAFETY: NSView's designated initializer; the class adds no ivars.
            unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: rect] }
        };
        document.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        content.setDocumentView(Some(&document));
        let content_controller = NSViewController::new(mtm);
        content_controller.setView(&content);
        let content_item = NSSplitViewItem::splitViewItemWithViewController(&content_controller);
        content_item.setMinimumThickness(480.0);

        let split = NSSplitViewController::new(mtm);
        split.view().setFrame(rect);
        split.addSplitViewItem(&sidebar_item);
        split.addSplitViewItem(&content_item);
        window.setContentViewController(Some(&split));
        window.setContentMinSize(NSSize::new(720.0, 420.0));
        window.setContentSize(rect.size);

        let contexts = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(180.0, 24.0)),
            false,
        );
        // SAFETY: The target implements inspectorContext: with the pop-up as the sender, and outlives the window.
        unsafe {
            NSControl::setTarget(&contexts, Some(target));
            NSControl::setAction(&contexts, Some(sel!(inspectorContext:)));
        }
        contexts.setAccessibilityLabel(Some(ns_string!("Context")));

        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("CicadaInspectorToolbar"));
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setDelegate(Some(ProtocolObject::from_ref(target)));
        window.setDelegate(Some(ProtocolObject::from_ref(target)));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        Self { window, sidebar, content, document, contexts, toolbar, _split: split }
    }
}

fn toolbar_identifiers() -> Retained<NSArray<NSToolbarItemIdentifier>> {
    // SAFETY: These are AppKit's constant identifiers, valid for the life of the process.
    let (tracking, flexible) =
        unsafe { (NSToolbarSidebarTrackingSeparatorItemIdentifier, NSToolbarFlexibleSpaceItemIdentifier) };
    let back = NSString::from_str(BACK);
    let forward = NSString::from_str(FORWARD);
    let context = NSString::from_str(CONTEXT);
    let refresh = NSString::from_str(REFRESH);
    NSArray::from_slice(&[tracking, &*back, &*forward, flexible, &*context, &*refresh])
}

/// A cell of the sidebar: a group's heading, or an item with its symbol.
fn sidebar_cell(mtm: MainThreadMarker, row: Row) -> Retained<NSView> {
    let cell = NSTableCellView::new(mtm);
    cell.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(200.0, 28.0)));
    let (title, symbol) = match row {
        Row::Group(title) => (title, None),
        Row::Item(section) => (section.title(), Some(section_symbol(section))),
    };
    let text = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    let x = if symbol.is_some() { 28.0 } else { 4.0 };
    text.setFrame(NSRect::new(NSPoint::new(x, 6.0), NSSize::new(200.0 - x - 4.0, 16.0)));
    text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    cell.addSubview(&text);
    // SAFETY: The text field is a subview of the cell, which keeps it.
    unsafe { cell.setTextField(Some(&text)) };
    if let Some(symbol) = symbol {
        let image = NSImageView::new(mtm);
        image.setImage(symbol_image(symbol, title).as_deref());
        image.setFrame(NSRect::new(NSPoint::new(4.0, 5.0), NSSize::new(18.0, 18.0)));
        cell.addSubview(&image);
        // SAFETY: The image view is a subview of the cell, which keeps it.
        unsafe { cell.setImageView(Some(&image)) };
    }
    Retained::into_super(cell)
}

fn section_symbol(section: Section) -> &'static str {
    match section {
        Section::Overview => "square.grid.2x2",
        Section::Actors => "person.2",
        Section::Messages => "tray",
        Section::RunningTurns => "play.circle",
        Section::Outcomes => "checkmark.circle",
        Section::Nodes => "server.rack",
        Section::Types => "doc.text",
        Section::Watches => "eye",
    }
}

fn symbol_image(symbol: &str, description: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        Some(&NSString::from_str(description)),
    )
}

fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: An action, when given, is one the item's target or the responder chain implements.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}

/// The main menu: Quit (⌘Q); Close (⌘W); Refresh (⌘R), Back (⌘[) and Forward (⌘]).
fn main_menu(mtm: MainThreadMarker, target: &Target) -> Retained<NSMenu> {
    let main = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
    let name = NSProcessInfo::processInfo().processName().to_string();
    let app = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
    app.addItem(&menu_item(mtm, &format!("Quit {name}"), Some(sel!(terminate:)), "q"));
    let file = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("File"));
    file.addItem(&menu_item(mtm, "Close", Some(sel!(performClose:)), "w"));
    let view = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("View"));
    for (title, action, key) in [
        ("Refresh", sel!(inspectorRefresh:), "r"),
        ("Back", sel!(inspectorBack:), "["),
        ("Forward", sel!(inspectorForward:), "]"),
    ] {
        let item = menu_item(mtm, title, Some(action), key);
        // SAFETY: The target implements the action and outlives the menu.
        unsafe { item.setTarget(Some(target)) };
        view.addItem(&item);
    }
    for (title, submenu) in [("App", app), ("File", file), ("View", view)] {
        let item = menu_item(mtm, title, None, "");
        item.setSubmenu(Some(&submenu));
        main.addItem(&item);
    }
    main
}

/// Ends `NSApplication::run` from inside an action.
fn stop_app(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
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
