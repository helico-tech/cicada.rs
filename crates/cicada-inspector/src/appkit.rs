//! Main-thread AppKit lifecycle: the window with its sidebar, toolbar and content, the main menu, and their actions.
#![allow(unsafe_code)]

mod actors;
mod content;
mod json_text;
mod messages;

use std::cell::{Cell, OnceCell, RefCell};
use std::process::ExitCode;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use cicada_app::Env;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSButton, NSControl, NSControlStateValueOn,
    NSControlTextEditingDelegate, NSEvent, NSEventModifierFlags, NSEventType, NSImage, NSImageView, NSMenu, NSMenuItem,
    NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate, NSPopUpButton, NSScrollView, NSSearchField,
    NSSplitViewController, NSSplitViewItem, NSTableCellView, NSTableColumn, NSTableColumnResizingOptions, NSTableView,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextField,
    NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem,
    NSToolbarItemIdentifier, NSToolbarSidebarTrackingSeparatorItemIdentifier, NSUserInterfaceItemIdentification,
    NSView, NSViewController, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWindowToolbarStyle,
};
use objc2_foundation::{
    NSArray, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer, ns_string,
};
use tokio::{runtime::Handle, sync::watch};

use crate::actors::{ListView, PaneView};
use crate::json::Json;
use crate::lifecycle::WindowSlot;
use crate::messages::{MessagePane, MessagesView};
use crate::model::{Limits, Navigation, Section, Session, View, check_macos};
use actors::{CellContent, Item, ItemId, Items, Page};
use content::FlippedView;
use messages::MessagesPage;

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
const REFRESH_KEY: &str = "r";
const SEARCH: &str = "inspector.search";
const PANE: &str = "inspector.pane";
/// The identifier of the Messages table, which shares the sidebar's data source and delegate.
const MESSAGES_TABLE: &str = "inspector.messages";

/// True for the pages with a table, the search field and the pane.
fn paged(section: Section) -> bool {
    matches!(section, Section::Actors | Section::Messages)
}

fn is_messages(table: &NSTableView) -> bool {
    table.identifier().is_some_and(|identifier| identifier.to_string() == MESSAGES_TABLE)
}

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
            // A new window gets its own generation and view channel, independent of any closed window.
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
    pane_toggle: RefCell<Option<Retained<NSToolbarItem>>>,
    /// The Actors page's rows and the pane's reads.
    list: RefCell<watch::Receiver<ListView>>,
    pane: RefCell<watch::Receiver<PaneView>>,
    /// The rows the table shows; the data source reads them.
    list_shown: RefCell<ListView>,
    /// One retained AppKit item per identity in the snapshot.
    items: RefCell<Items>,
    /// The state the pane's text view holds.
    state_shown: RefCell<Option<Arc<Json>>>,
    /// The Messages page's rows and pane.
    messages_list: RefCell<watch::Receiver<MessagesView>>,
    messages_pane: RefCell<watch::Receiver<MessagePane>>,
    /// The messages the table shows; the data source reads them.
    messages_shown: RefCell<MessagesView>,
    /// The payload the Messages pane's text view holds.
    payload_shown: RefCell<Option<Arc<Json>>>,
    /// Whether the pane shows on the Actors and Messages pages; elsewhere the pane is collapsed.
    pane_shown: Cell<bool>,
    /// The section last rendered, to place the pane only when the page changes.
    rendered: Cell<Option<Section>>,
    /// What the context pop-up shows, so that it is rebuilt only when that changes.
    contexts_shown: RefCell<Option<ContextsShown>>,
    /// True while the sidebar or outline is updated from the model. Callbacks then ignore selection and folds.
    syncing: Cell<bool>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    finished: Cell<bool>,
}

define_class!(
    // SAFETY: NSWindow may be subclassed, and the subclass adds no ivars or Drop.
    #[unsafe(super(NSWindow, objc2_app_kit::NSResponder, NSObject))]
    #[name = "CicadaInspectorWindow"]
    #[thread_kind = MainThreadOnly]
    struct InspectorWindow;

    impl InspectorWindow {
        // SAFETY: The signature is NSResponder's performKeyEquivalent:.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            // SAFETY: Forwards the event unchanged to NSWindow.
            self.handle_key(event) || unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }
    }
);

impl InspectorWindow {
    fn handle_key(&self, event: &NSEvent) -> bool {
        let modifiers = NSEventModifierFlags::Command
            | NSEventModifierFlags::Shift
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Control;
        if !self.isKeyWindow()
            || event.r#type() != NSEventType::KeyDown
            || event.modifierFlags() & modifiers != NSEventModifierFlags::Command
        {
            return false;
        }
        let Some(key) = event.charactersIgnoringModifiers() else { return false };
        let Some(delegate) = self.delegate() else { return false };
        let Some(target) = AsRef::<AnyObject>::as_ref(&*delegate).downcast_ref::<Target>() else { return false };

        match key.to_string().to_ascii_lowercase().as_str() {
            "w" => self.performClose(None),
            REFRESH_KEY => target.refresh_view(),
            "[" => target.change_history(Navigation::back),
            "]" => target.change_history(Navigation::forward),
            _ => return false,
        }
        // A handled key must not reach the toolbar's overflow or the standalone app's menu again.
        true
    }
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
            let c = self.ivars();
            if !c.finished.get() && let Some(window) = c.window.get() {
                let view = c.view.borrow().borrow().clone();
                let section = c.navigation.borrow().current();
                content::render(&window.document, &window.content, section, &view);
                if section == Section::Actors {
                    self.render_pane();
                    self.check_more();
                } else if section == Section::Messages {
                    let list = c.messages_shown.borrow().clone();
                    window.messages.show_footer(&list, view.target_error.as_deref());
                    self.render_message_pane();
                    self.check_more_messages();
                }
            }
        }
    }

    // SAFETY: NSControlTextEditingDelegate has only optional methods.
    unsafe impl NSControlTextEditingDelegate for Target {}

    // SAFETY: The callback has the signature required by NSTableViewDataSource.
    unsafe impl NSTableViewDataSource for Target {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, table: &NSTableView) -> NSInteger {
            if is_messages(table) {
                self.ivars().messages_shown.borrow().rows.len() as NSInteger
            } else {
                ROWS as NSInteger
            }
        }
    }

    // SAFETY: These callbacks have the signatures required by NSTableViewDelegate and are used by the sidebar and the
    // Messages table, told apart by the table's identifier.
    unsafe impl NSTableViewDelegate for Target {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            table: &NSTableView,
            column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            if is_messages(table) {
                let column = column.map(|column| column.identifier().to_string()).unwrap_or_default();
                let text = messages::cell_text(&self.ivars().messages_shown.borrow(), row, &column);
                text.map(|text| messages::cell(self.mtm(), &text, &column))
            } else {
                row_at(row).map(|row| sidebar_cell(self.mtm(), row))
            }
        }

        #[unsafe(method(tableView:isGroupRow:))]
        fn is_group_row(&self, table: &NSTableView, row: NSInteger) -> bool {
            !is_messages(table) && matches!(row_at(row), Some(Row::Group(_)))
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select_row(&self, table: &NSTableView, row: NSInteger) -> bool {
            is_messages(table) || matches!(row_at(row), Some(Row::Item(_)))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            let c = self.ivars();
            if c.syncing.get() || c.finished.get() {
                return;
            }
            let window = self.window();
            let is = |view: &AnyObject| notification.object().is_some_and(|object| std::ptr::eq(&*object, view));
            if is(window.sidebar.as_ref()) {
                if let Some(Row::Item(section)) = row_at(window.sidebar.selectedRow()) {
                    self.navigate(|navigation| {
                        let before = navigation.current();
                        navigation.select(section);
                        navigation.current() != before
                    });
                }
            } else if is(window.messages.table.as_ref()) {
                let row = usize::try_from(window.messages.table.selectedRow()).ok();
                let id = row.and_then(|row| c.messages_shown.borrow().rows.get(row).map(|message| message.id.clone()));
                c.session.messages().select(id.as_ref());
                self.render_message_pane();
            }
        }
    }

    // SAFETY: The callbacks conform to NSOutlineViewDataSource. A nonnil item is one returned from outline_child.
    unsafe impl NSOutlineViewDataSource for Target {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn outline_count(&self, _: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            let parent = item.map(|item| Item::id(item).expect("the outline owns its item identities"));
            actors::child_count(&self.ivars().list_shown.borrow(), parent) as NSInteger
        }

        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn outline_child(&self, _: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            let c = self.ivars();
            let parent = item.map(|item| Item::id(item).expect("the outline owns its item identities"));
            let id = actors::child_at(&c.list_shown.borrow(), parent, index as usize);
            c.items.borrow()[&id].clone().into()
        }

        #[unsafe(method(outlineView:isItemExpandable:))]
        fn outline_can_expand(&self, _: &NSOutlineView, item: &AnyObject) -> bool {
            matches!(Item::id(item), Some(ItemId::Actor(_)))
        }
    }

    // SAFETY: The callbacks have the signatures required by NSOutlineViewDelegate and handle only its own items.
    unsafe impl NSOutlineViewDelegate for Target {
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn outline_cell(
            &self,
            _: &NSOutlineView,
            column: Option<&NSTableColumn>,
            item: &AnyObject,
        ) -> Option<Retained<NSView>> {
            let column = column.map(|column| column.identifier().to_string());
            Item::id(item).and_then(|item| {
                let content = CellContent::of(&self.ivars().list_shown.borrow(), item, column.as_deref())?;
                Some(content.view(
                    self.mtm(), column.as_deref(), self.as_ref(), sel!(inspectorRetry:), sel!(inspectorMoreChildren:),
                ))
            })
        }

        #[unsafe(method(outlineView:isGroupItem:))]
        fn outline_is_group(&self, _: &NSOutlineView, item: &AnyObject) -> bool {
            matches!(Item::id(item), Some(ItemId::Status(None)))
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn outline_can_select(&self, _: &NSOutlineView, item: &AnyObject) -> bool {
            matches!(Item::id(item), Some(ItemId::Actor(_)))
        }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn outline_selection_changed(&self, _: &NSNotification) {
            let c = self.ivars();
            if c.syncing.get() || c.finished.get() {
                return;
            }
            let table = &self.window().page.table;
            let row = table.selectedRow();
            let item = (row >= 0).then(|| table.itemAtRow(row)).flatten();
            let path = match item.as_deref().and_then(Item::id) {
                Some(ItemId::Actor(path)) => Some(path.clone()),
                _ => None,
            };
            c.session.actors().select(path);
            self.render_pane();
        }

        #[unsafe(method(outlineViewItemDidExpand:))]
        fn outline_opened(&self, notification: &NSNotification) {
            self.fold_changed(notification, true);
        }

        #[unsafe(method(outlineViewItemDidCollapse:))]
        fn outline_closed(&self, notification: &NSNotification) {
            self.fold_changed(notification, false);
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
            let c = self.ivars();
            if c.view.borrow().has_changed().unwrap_or(false) {
                self.render();
            }
            let section = c.navigation.borrow().current();
            if section == Section::Actors {
                if c.list.borrow().has_changed().unwrap_or(false) {
                    self.render_list();
                }
                if c.pane.borrow().has_changed().unwrap_or(false) {
                    self.render_pane();
                }
                self.check_more();
            } else if section == Section::Messages {
                if c.messages_list.borrow().has_changed().unwrap_or(false) {
                    self.render_messages();
                }
                if c.messages_pane.borrow().has_changed().unwrap_or(false) {
                    self.render_message_pane();
                }
                self.check_more_messages();
            }
        }

        // SAFETY: The search field sends this action with itself as the sender, at every change of its text.
        #[unsafe(method(inspectorSearch:))]
        fn search(&self, sender: &NSSearchField) {
            let c = self.ivars();
            if c.finished.get() {
                return;
            }
            // A field that sends the same text again reads nothing.
            // Every borrow ends before AppKit renders, which can call back into the target.
            let prefix = sender.stringValue().to_string();
            let section = c.navigation.borrow().current();
            let searched = match section {
                Section::Actors => c.list.borrow().borrow().prefix.clone(),
                Section::Messages => c.messages_list.borrow().borrow().prefix.clone(),
                _ => return,
            };
            if searched == prefix {
                return;
            }
            if section == Section::Actors {
                c.session.actors().search(&prefix);
                self.render_list();
                self.render_pane();
            } else {
                c.session.messages().search(&prefix);
                self.render_messages();
                self.render_message_pane();
            }
        }

        // SAFETY: The Retry button of the Messages footer sends this action with itself as the sender.
        #[unsafe(method(inspectorMessagesRetry:))]
        fn messages_retry(&self, _: &NSButton) {
            if !self.ivars().finished.get() {
                self.ivars().session.messages().retry();
            }
        }

        // SAFETY: The Show Actor button of the Messages pane sends this action with itself as the sender.
        #[unsafe(method(inspectorShowActor:))]
        fn show_actor(&self, _: &NSButton) {
            let c = self.ivars();
            if c.finished.get() {
                return;
            }
            let to = c.messages_pane.borrow().borrow().message.as_ref().map(|message| message.to.clone());
            if let Some(to) = to {
                c.session.show_actor(&mut c.navigation.borrow_mut(), to);
                self.sync_search();
                self.render();
            }
        }

        // SAFETY: The Retry buttons of outline status rows send this action with themselves as the sender.
        #[unsafe(method(inspectorRetry:))]
        fn retry(&self, sender: &NSButton) {
            let c = self.ivars();
            if c.finished.get() {
                return;
            }
            let item = self.item_for_button(sender);
            let generation = c.list_shown.borrow().generation;
            match item.as_deref().and_then(Item::id) {
                Some(ItemId::Status(None)) => c.session.actors().retry(),
                Some(ItemId::Status(Some(parent))) if c.session.actors().retry_children(generation, parent) => {
                    sender.setEnabled(false);
                }
                _ => {}
            }
        }

        // SAFETY: The more… buttons of outline status rows send this action with themselves as the sender.
        #[unsafe(method(inspectorMoreChildren:))]
        fn more_children(&self, sender: &NSButton) {
            let c = self.ivars();
            if c.finished.get() {
                return;
            }
            let item = self.item_for_button(sender);
            let generation = c.list_shown.borrow().generation;
            if let Some(ItemId::Status(Some(parent))) = item.as_deref().and_then(Item::id)
                && c.session.actors().more_children(generation, parent)
            {
                sender.setEnabled(false);
            }
        }

        // SAFETY: The pane toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorTogglePane:))]
        fn toggle_pane(&self, _: Option<&AnyObject>) {
            let c = self.ivars();
            if c.finished.get() || !paged(c.navigation.borrow().current()) {
                return;
            }
            // From the pane's own state: the user may also have dragged it shut.
            let show = self.window().pane_item.isCollapsed();
            c.pane_shown.set(show);
            self.collapse_pane(!show);
            self.render_pane();
        }

        // SAFETY: The Back toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorBack:))]
        fn back(&self, _: Option<&AnyObject>) {
            self.change_history(Navigation::back);
        }

        // SAFETY: The Forward toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorForward:))]
        fn forward(&self, _: Option<&AnyObject>) {
            self.change_history(Navigation::forward);
        }

        // SAFETY: The Refresh toolbar item and menu item send this action with themselves as the sender.
        #[unsafe(method(inspectorRefresh:))]
        fn refresh(&self, _: Option<&AnyObject>) {
            self.refresh_view();
        }

        // SAFETY: Only the context NSPopUpButton is wired to this action.
        #[unsafe(method(inspectorContext:))]
        fn pick_context(&self, sender: &NSPopUpButton) {
            if self.ivars().finished.get() {
                return;
            }
            let Some(item) = sender.selectedItem() else { return };
            let name = item.title().to_string();
            let c = self.ivars();
            c.session.switch_context(&mut c.navigation.borrow_mut(), Some(name));
            self.render();
        }

        /// Back, Forward and Refresh in the menu are enabled as their toolbar items are.
        // SAFETY: The signature is NSMenuItemValidation's validateMenuItem:.
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let action = item.action();
            let (can_back, can_forward, section) = {
                let navigation = self.ivars().navigation.borrow();
                (navigation.can_back(), navigation.can_forward(), navigation.current())
            };
            match action {
                Some(action) if action == sel!(inspectorBack:) => can_back,
                Some(action) if action == sel!(inspectorForward:) => can_forward,
                Some(action) if action == sel!(inspectorRefresh:) => self.can_refresh(),
                Some(action) if action == sel!(inspectorTogglePane:) => {
                    let collapsed = self.window().pane_item.isCollapsed();
                    item.setTitle(if collapsed { ns_string!("Show Pane") } else { ns_string!("Hide Pane") });
                    paged(section)
                }
                _ => true,
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, session: Session, owner: Option<Weak<Windows>>) -> Retained<Self> {
        let view = session.view();
        let (list, pane) = (session.actors().list(), session.actors().pane());
        let (messages_list, messages_pane) = (session.messages().list(), session.messages().pane());
        let allocated = Self::alloc(mtm).set_ivars(Controller {
            session,
            owner,
            view: RefCell::new(view),
            navigation: RefCell::default(),
            window: OnceCell::new(),
            back: RefCell::default(),
            forward: RefCell::default(),
            refresh: RefCell::default(),
            pane_toggle: RefCell::default(),
            list: RefCell::new(list),
            pane: RefCell::new(pane),
            list_shown: RefCell::default(),
            items: RefCell::new(Items::from([(ItemId::Status(None), Item::new(mtm, ItemId::Status(None)))])),
            state_shown: RefCell::default(),
            messages_list: RefCell::new(messages_list),
            messages_pane: RefCell::new(messages_pane),
            messages_shown: RefCell::default(),
            payload_shown: RefCell::default(),
            pane_shown: Cell::new(true),
            rendered: Cell::new(None),
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
        let window = &self.window().window;
        if !self.ivars().finished.get() && window.isMiniaturized() {
            window.deminiaturize(None);
        }
        if !self.ivars().finished.get() {
            window.makeKeyAndOrderFront(None);
        }
    }

    fn window(&self) -> &Window {
        self.ivars().window.get().expect("the window is made before the app runs")
    }

    fn can_refresh(&self) -> bool {
        let c = self.ivars();
        !c.finished.get() && c.view.borrow().borrow().target_error.is_none()
    }

    /// ⌘R: the Actors page reloads its list from the top and its pane, the Messages page its list; another page
    /// refreshes the Overview.
    fn refresh_view(&self) {
        if self.can_refresh() {
            let c = self.ivars();
            let section = c.navigation.borrow().current();
            match section {
                Section::Actors => c.session.actors().refresh(),
                Section::Messages => c.session.messages().refresh(),
                _ => c.session.refresh(),
            }
            self.render();
        }
    }

    fn item_for_button(&self, sender: &NSButton) -> Option<Retained<AnyObject>> {
        let table = &self.window().page.table;
        let row = table.rowForView(sender);
        (row >= 0).then(|| table.itemAtRow(row)).flatten()
    }

    fn fold_changed(&self, notification: &NSNotification, open: bool) {
        let c = self.ivars();
        if c.syncing.get() || c.finished.get() {
            return;
        }
        let item = notification.userInfo().and_then(|info| info.objectForKey(ns_string!("NSObject").as_ref()));
        let Some(ItemId::Actor(path)) = item.as_deref().and_then(Item::id) else { return };
        let generation = c.list_shown.borrow().generation;
        if open {
            c.session.actors().open(generation, path);
        } else {
            c.session.actors().close(generation, path);
        }
    }

    fn collapse_pane(&self, collapsed: bool) {
        let window = self.window();
        if window.pane_item.isCollapsed() != collapsed {
            window.pane_item.setCollapsed(collapsed);
            // The pane is laid out from its width, so the split view places it first.
            if let Some(view) = window.window.contentView() {
                view.layoutSubtreeIfNeeded();
            }
        }
    }

    /// Reloads the outline, reopens its loaded paths and restores selection by identity.
    fn render_list(&self) {
        let c = self.ivars();
        if c.finished.get() || c.syncing.get() {
            return;
        }
        let Some(window) = c.window.get() else { return };
        let list = c.list.borrow_mut().borrow_and_update().clone();
        let target_error = c.view.borrow().borrow().target_error.clone();
        window.page.show_footer(&list, target_error.as_deref());
        let selected = c.session.actors().selected().map(ItemId::Actor);
        let mut previous = c.items.take();
        let items = actors::items(self.mtm(), &list, &mut previous);
        let selected = selected.as_ref().and_then(|id| items.get(id)).cloned();
        let (open, closed): (Vec<_>, Vec<_>) = items
            .iter()
            .filter_map(|(id, item)| match id {
                ItemId::Actor(path) => Some((list.open.contains(path), item.clone())),
                ItemId::Status(_) => None,
            })
            .partition(|(open, _)| *open);
        c.items.replace(items);
        c.list_shown.replace(list);
        let table = &window.page.table;
        c.syncing.set(true);
        table.reloadData();
        drop(previous);
        // SAFETY: These items belong to the reloaded outline and stay retained throughout the calls. The syncing
        // guard ignores the callbacks from programmatic folding and selection.
        unsafe {
            for (_, item) in &closed {
                if table.isItemExpanded(Some(item.as_ref())) {
                    table.collapseItem(Some(item.as_ref()));
                }
            }
            // The paths are sorted, so each open ancestor is expanded before any of its descendants.
            for (_, item) in &open {
                let item = Some(item.as_ref());
                if table.rowForItem(item) >= 0 && !table.isItemExpanded(item) {
                    table.expandItem(item);
                }
            }
            let row = selected.as_ref().map_or(-1, |item| table.rowForItem(Some(item.as_ref())));
            if let Ok(row) = usize::try_from(row) {
                table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false);
            } else {
                table.deselectAll(None);
            }
        }
        c.syncing.set(false);
        self.check_more();
    }

    /// Asks for a page when the last visible root row is near the end, including after scrolling.
    fn check_more(&self) {
        let c = self.ivars();
        let Some(window) = c.window.get() else { return };
        let Some(last) = window.page.last_visible_root() else { return };
        let list = c.list_shown.borrow();
        let wants = list.root_index(last.address()).is_some_and(|index| list.wants_more(index));
        drop(list);
        if wants {
            c.session.actors().more();
        }
    }

    /// Shows the latest reads of the selected actor.
    fn render_pane(&self) {
        let c = self.ivars();
        if c.finished.get() {
            return;
        }
        let Some(window) = c.window.get() else { return };
        let pane = c.pane.borrow_mut().borrow_and_update().clone();
        let shown = c.state_shown.borrow().clone();
        let shown = window.page.show_pane(&pane, shown.as_ref());
        c.state_shown.replace(shown);
    }

    fn change_history(&self, change: fn(&mut Navigation) -> bool) {
        self.navigate(change);
    }

    /// Moves in the history through the session, which starts or cancels a Show Actor; then the search field shows
    /// the page's prefix and the window the new place.
    fn navigate(&self, change: impl FnOnce(&mut Navigation) -> bool) {
        let c = self.ivars();
        if c.finished.get() {
            return;
        }
        let changed = c.session.navigate(&mut c.navigation.borrow_mut(), change);
        if changed {
            self.sync_search();
            self.render();
        }
    }

    /// The search field shows the prefix of the page it searches, and says what it searches.
    fn sync_search(&self) {
        let c = self.ivars();
        let section = c.navigation.borrow().current();
        let (prefix, tip) = match section {
            Section::Actors => (c.list.borrow().borrow().prefix.clone(), "Search actors by address prefix"),
            Section::Messages => (
                c.messages_list.borrow().borrow().prefix.clone(),
                "Search pending messages by their actor's address prefix",
            ),
            _ => return,
        };
        let search = &self.window().search;
        if search.stringValue().to_string() != prefix {
            search.setStringValue(&NSString::from_str(&prefix));
        }
        search.setToolTip(Some(&NSString::from_str(tip)));
        search.setAccessibilityLabel(Some(&NSString::from_str(tip)));
    }

    /// Reloads the Messages table and restores the selection by message id.
    fn render_messages(&self) {
        let c = self.ivars();
        if c.finished.get() || c.syncing.get() {
            return;
        }
        let Some(window) = c.window.get() else { return };
        let list = c.messages_list.borrow_mut().borrow_and_update().clone();
        let target_error = c.view.borrow().borrow().target_error.clone();
        window.messages.show_footer(&list, target_error.as_deref());
        let selected = c.session.messages().selected().and_then(|id| list.index(&id));
        c.messages_shown.replace(list);
        let table = &window.messages.table;
        c.syncing.set(true);
        table.reloadData();
        match selected {
            Some(row) => table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false),
            // SAFETY: A nil sender is allowed; the syncing guard ignores the selection callback.
            None => unsafe { table.deselectAll(None) },
        }
        c.syncing.set(false);
        self.check_more_messages();
    }

    /// Asks for a page of messages when the last visible row is near the end, including after scrolling.
    fn check_more_messages(&self) {
        let c = self.ivars();
        let Some(window) = c.window.get() else { return };
        let last = window.messages.last_visible_row().unwrap_or(0);
        let wants = c.messages_shown.borrow().wants_more(last);
        if wants {
            c.session.messages().more();
        }
    }

    /// Shows the selected message.
    fn render_message_pane(&self) {
        let c = self.ivars();
        if c.finished.get() {
            return;
        }
        let Some(window) = c.window.get() else { return };
        let pane = c.messages_pane.borrow_mut().borrow_and_update().clone();
        let shown = c.payload_shown.borrow().clone();
        let shown = window.messages.show_pane(&pane, shown.as_ref());
        c.payload_shown.replace(shown);
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
        let (actors, messages, has_pane) = (section == Section::Actors, section == Section::Messages, paged(section));
        for (item, enabled) in [
            (&c.back, can_back),
            (&c.forward, can_forward),
            (&c.refresh, view.target_error.is_none()),
            (&c.pane_toggle, has_pane),
        ] {
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
        window.search.setEnabled(has_pane);
        window.content.setHidden(has_pane);
        window.page.root.setHidden(!actors);
        window.page.pane.setHidden(!actors);
        window.messages.root.setHidden(!messages);
        window.messages.pane.setHidden(!messages);
        let before = c.rendered.replace(Some(section));
        if before != Some(section) {
            if before.is_some_and(paged) {
                c.pane_shown.set(!window.pane_item.isCollapsed());
            }
            self.collapse_pane(!(has_pane && c.pane_shown.get()));
        }
        content::render(&window.document, &window.content, section, &view);
        if actors {
            c.session.actors().show();
            self.render_list();
            self.render_pane();
        } else if messages {
            c.session.messages().show();
            self.render_messages();
            self.render_message_pane();
        }
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
                let overflow = menu_item(mtm, "Refresh", Some(sel!(inspectorRefresh:)), REFRESH_KEY);
                // SAFETY: This is the same action and target as the toolbar item.
                unsafe { overflow.setTarget(Some(self)) };
                item.setMenuFormRepresentation(Some(&overflow));
                c.refresh.replace(Some(item.clone()));
            }
            CONTEXT => {
                item.setLabel(ns_string!("Context"));
                item.setToolTip(Some(ns_string!("The context of cicada config")));
                item.setView(Some(&self.window().contexts));
            }
            SEARCH => {
                item.setLabel(ns_string!("Search"));
                item.setToolTip(Some(ns_string!("Search by address prefix")));
                item.setView(Some(&self.window().search));
            }
            PANE => {
                button("sidebar.right", "Show or Hide Pane", sel!(inspectorTogglePane:));
                c.pane_toggle.replace(Some(item.clone()));
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
        // SAFETY: Clears the table's and the search field's references to the target while it is still alive.
        unsafe {
            window.sidebar.setDataSource(None);
            window.sidebar.setDelegate(None);
            NSControl::setTarget(&window.search, None);
        }
        window.page.clear_actions();
        window.messages.clear_actions();
        true
    }
}

/// The window's subtitle: the context in use, and the variables that win over it.
fn subtitle(view: &View) -> String {
    let context = view.target_label();
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
    search: Retained<NSSearchField>,
    /// The Actors page, next to the Overview's scroll view in the content.
    page: Page,
    /// The Messages page, next to them.
    messages: MessagesPage,
    /// The inspector pane's split item, collapsed outside the Actors page and by the pane toggle.
    pane_item: Retained<NSSplitViewItem>,
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
        let window: Retained<InspectorWindow> = unsafe {
            msg_send![
                InspectorWindow::alloc(mtm),
                initWithContentRect: rect,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
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
        // The content holds the Overview's scroll view and the Actors page; one of them shows.
        let fill = NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable;
        let container =
            NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, NSSize::new(760.0, 600.0)));
        let page = Page::new(mtm);
        let messages =
            MessagesPage::new(mtm, target.as_ref(), sel!(inspectorMessagesRetry:), sel!(inspectorShowActor:));
        for view in [&**content, &*page.root, &*messages.root] {
            view.setFrame(container.bounds());
            view.setAutoresizingMask(fill);
            container.addSubview(view);
        }
        page.root.setHidden(true);
        messages.root.setHidden(true);
        messages.table.setIdentifier(Some(&NSString::from_str(MESSAGES_TABLE)));
        // SAFETY: The target implements both protocols and clears them during teardown.
        unsafe {
            page.table.setDataSource(Some(ProtocolObject::from_ref(target)));
            page.table.setDelegate(Some(ProtocolObject::from_ref(target)));
            messages.table.setDataSource(Some(ProtocolObject::from_ref(target)));
            messages.table.setDelegate(Some(ProtocolObject::from_ref(target)));
        }
        let content_controller = NSViewController::new(mtm);
        content_controller.setView(&container);
        let content_item = NSSplitViewItem::splitViewItemWithViewController(&content_controller);
        content_item.setMinimumThickness(420.0);
        // The pane holds the Actors page's pane and the Messages page's; the page shown shows its own.
        let panes = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, NSSize::new(320.0, 600.0)));
        for view in [&**page.pane, &**messages.pane] {
            view.setFrame(panes.bounds());
            view.setAutoresizingMask(fill);
            panes.addSubview(view);
        }
        messages.pane.setHidden(true);
        let pane_controller = NSViewController::new(mtm);
        pane_controller.setView(&panes);
        let pane_item = NSSplitViewItem::inspectorWithViewController(&pane_controller);
        pane_item.setMinimumThickness(280.0);
        pane_item.setCanCollapse(true);
        pane_item.setCollapsed(true);

        let split = NSSplitViewController::new(mtm);
        split.view().setFrame(rect);
        split.addSplitViewItem(&sidebar_item);
        split.addSplitViewItem(&content_item);
        split.addSplitViewItem(&pane_item);
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
        let search = NSSearchField::initWithFrame(
            NSSearchField::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(200.0, 24.0)),
        );
        search.setPlaceholderString(Some(ns_string!("Address prefix")));
        search.setSendsSearchStringImmediately(true);
        search.setAccessibilityLabel(Some(ns_string!("Search by address prefix")));
        // SAFETY: The target implements inspectorSearch: with the field as the sender, and clears it during teardown.
        unsafe {
            NSControl::setTarget(&search, Some(target));
            NSControl::setAction(&search, Some(sel!(inspectorSearch:)));
        }

        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("CicadaInspectorToolbar"));
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setDelegate(Some(ProtocolObject::from_ref(target)));
        window.setDelegate(Some(ProtocolObject::from_ref(target)));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        window.setAutorecalculatesKeyViewLoop(true);
        Self {
            window: Retained::into_super(window),
            sidebar,
            content,
            document,
            contexts,
            search,
            page,
            messages,
            pane_item,
            toolbar,
            _split: split,
        }
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
    let search = NSString::from_str(SEARCH);
    let pane = NSString::from_str(PANE);
    NSArray::from_slice(&[tracking, &*back, &*forward, flexible, &*search, &*context, &*refresh, &*pane])
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
        ("Refresh", sel!(inspectorRefresh:), REFRESH_KEY),
        ("Back", sel!(inspectorBack:), "["),
        ("Forward", sel!(inspectorForward:), "]"),
        ("Hide Pane", sel!(inspectorTogglePane:), ""),
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
