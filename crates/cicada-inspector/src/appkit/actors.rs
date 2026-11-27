//! The Actors page: the table of actors with its footer, and the inspector pane of the selected actor with its tabs.
//! The target owns the data and the actions; this module builds the views and fills them.

use std::collections::BTreeMap;

use cicada::time::Timestamp;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSColor, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSLineBreakMode, NSOutlineView, NSScrollView, NSTabView, NSTabViewItem,
    NSTableColumn, NSTableColumnResizingOptions, NSTableViewColumnAutoresizingStyle, NSTableViewStyle, NSTextField,
    NSTextView, NSView,
};
use objc2_foundation::{NSInteger, NSObject, NSPoint, NSRange, NSRect, NSSize, NSString, ns_string};

use super::content::{FlippedView, Layout, Style, color, label};
use crate::actors::{ActorPath, ListView, MAILBOX_ROWS, Paging, PaneView, Read, WATCH_ROWS};
use crate::json::{Json, Token, utf16_ranges};
use crate::model::age;

/// The table's columns: identifier, title and width.
const COLUMNS: [(&str, &str, f64); 5] = [
    ("address", "Address", 260.0),
    ("type", "Type", 120.0),
    ("version", "Version", 70.0),
    ("created", "Created", 150.0),
    ("status", "Status", 120.0),
];
const FOOTER: f64 = 26.0;
const PANE_MARGIN: f64 = 16.0;

/// The identity of one row in the outline. A paging row stays in place as its text or action changes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ItemId {
    Actor(ActorPath),
    Status(Option<ActorPath>),
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Item does not implement Drop.
    #[unsafe(super = NSObject)]
    #[name = "CicadaInspectorOutlineItem"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ItemId]
    pub(super) struct Item;
);

impl Item {
    pub(super) fn new(mtm: MainThreadMarker, id: ItemId) -> Retained<Self> {
        let item = Self::alloc(mtm).set_ivars(id);
        // SAFETY: The ivars are set, and NSObject's init has no additional requirements.
        unsafe { msg_send![super(item), init] }
    }

    pub(super) fn id(item: &AnyObject) -> Option<&ItemId> {
        item.downcast_ref::<Self>().map(DefinedClass::ivars)
    }
}

pub(super) type Items = BTreeMap<ItemId, Retained<Item>>;

/// The objects for a new snapshot. Surviving identities keep their object. The caller keeps the remaining
/// `previous` objects alive through reloadData.
pub(super) fn items(mtm: MainThreadMarker, list: &ListView, previous: &mut Items) -> Items {
    let roots = list.rows.iter().map(|row| ActorPath::root(row.address.clone()));
    let children =
        list.children.iter().flat_map(|(parent, page)| page.rows.iter().map(|address| parent.child(address.clone())));
    let mut items = Items::new();
    for path in roots.chain(children) {
        if child_status_row(list, &path).is_some() {
            let id = ItemId::Status(Some(path.clone()));
            items.insert(id.clone(), previous.remove(&id).unwrap_or_else(|| Item::new(mtm, id)));
        }
        let actor = ItemId::Actor(path);
        items.insert(actor.clone(), previous.remove(&actor).unwrap_or_else(|| Item::new(mtm, actor)));
    }
    if root_status_row(list).is_some() {
        let id = ItemId::Status(None);
        items.insert(id.clone(), previous.remove(&id).unwrap_or_else(|| Item::new(mtm, id)));
    }
    items
}

pub(super) fn child_count(list: &ListView, item: Option<&ItemId>) -> usize {
    match item {
        None => list.rows.len() + usize::from(root_status_row(list).is_some()),
        Some(ItemId::Actor(path)) => {
            list.children.get(path).map_or(0, |page| page.rows.len())
                + usize::from(child_status_row(list, path).is_some())
        }
        Some(ItemId::Status(_)) => 0,
    }
}

pub(super) fn child_at(list: &ListView, item: Option<&ItemId>, index: usize) -> ItemId {
    match item {
        None => {
            list.rows.get(index).map_or(ItemId::Status(None), |row| ItemId::Actor(ActorPath::root(row.address.clone())))
        }
        Some(ItemId::Actor(path)) => list
            .children
            .get(path)
            .and_then(|page| page.rows.get(index))
            .map_or_else(|| ItemId::Status(Some(path.clone())), |address| ItemId::Actor(path.child(address.clone()))),
        Some(ItemId::Status(_)) => unreachable!("a status row has no children"),
    }
}

enum StatusRow<'a> {
    Loading,
    Failed(&'a str),
    NoActors,
    NoChildren,
    NotFound,
    More { loading: bool },
}

fn root_status_row(list: &ListView) -> Option<StatusRow<'_>> {
    match &list.paging {
        Paging::Loading => Some(StatusRow::Loading),
        Paging::Failed(error) => Some(StatusRow::Failed(error)),
        Paging::Idle if list.rows.is_empty() && !list.more => Some(StatusRow::NoActors),
        Paging::Idle => None,
    }
}

fn child_status_row<'a>(list: &'a ListView, parent: &ActorPath) -> Option<StatusRow<'a>> {
    let Some(page) = list.children.get(parent) else { return Some(StatusRow::Loading) };
    match &page.paging {
        Paging::Failed(error) => Some(StatusRow::Failed(error)),
        paging if page.more => Some(StatusRow::More { loading: *paging == Paging::Loading }),
        Paging::Loading => Some(StatusRow::Loading),
        Paging::Idle if page.not_found => Some(StatusRow::NotFound),
        Paging::Idle if page.rows.is_empty() => Some(StatusRow::NoChildren),
        Paging::Idle => None,
    }
}

/// The views of the Actors page and its pane.
pub(super) struct Page {
    /// The table with the footer under it.
    pub root: Retained<NSView>,
    pub table: Retained<NSOutlineView>,
    footer: Retained<NSTextField>,
    /// The pane's root, laid out from the top.
    pub pane: Retained<FlippedView>,
    header: Retained<FlippedView>,
    tabs: Retained<NSTabView>,
    state: Retained<NSView>,
    state_notice: Retained<NSTextField>,
    state_scroll: Retained<NSScrollView>,
    state_text: Retained<NSTextView>,
    mailbox: Retained<FlippedView>,
    mailbox_scroll: Retained<NSScrollView>,
    watches: Retained<FlippedView>,
    watches_scroll: Retained<NSScrollView>,
    /// The colours of the tokens, made once.
    colors: [Retained<NSColor>; 5],
}

impl Page {
    pub(super) fn new(mtm: MainThreadMarker) -> Self {
        let size = NSSize::new(760.0, 600.0);
        let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
        let table = NSOutlineView::new(mtm);
        for (identifier, title, width) in COLUMNS {
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str(identifier));
            column.setTitle(&NSString::from_str(title));
            column.setWidth(width);
            column.setMinWidth(50.0);
            column.setResizingMask(NSTableColumnResizingOptions::UserResizingMask);
            table.addTableColumn(&column);
            if identifier == "address" {
                // SAFETY: This is a column already added to this outline.
                unsafe { table.setOutlineTableColumn(Some(&column)) };
            }
        }
        table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle);
        table.setStyle(NSTableViewStyle::Inset);
        table.setUsesAlternatingRowBackgroundColors(true);
        table.setFloatsGroupRows(false);
        table.setAllowsEmptySelection(true);
        table.setRowHeight(22.0);
        table.setAccessibilityLabel(Some(ns_string!("Actors")));
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, FOOTER), NSSize::new(size.width, size.height - FOOTER)),
        );
        scroll.setDocumentView(Some(&table));
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        root.addSubview(&scroll);
        let footer = label(mtm, "", NSFont::systemFontOfSize(11.0), Style::Secondary);
        footer.setFrame(NSRect::new(NSPoint::new(12.0, 5.0), NSSize::new(size.width - 24.0, 16.0)));
        footer.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        root.addSubview(&footer);

        let pane_size = NSSize::new(320.0, 600.0);
        // SAFETY: NSView's designated initializer; the class adds no ivars.
        let pane: Retained<FlippedView> =
            unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, pane_size)] };
        pane.setAccessibilityLabel(Some(ns_string!("Inspector pane")));
        // SAFETY: as above.
        let header: Retained<FlippedView> =
            unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, pane_size)] };
        header.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        pane.addSubview(&header);
        let tabs = NSTabView::initWithFrame(NSTabView::alloc(mtm), NSRect::new(NSPoint::ZERO, pane_size));
        tabs.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        pane.addSubview(&tabs);

        // State: the cut notice over the text.
        let state = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, pane_size));
        let state_scroll = NSTextView::scrollableTextView(mtm);
        state_scroll.setFrame(NSRect::new(NSPoint::ZERO, pane_size));
        state_scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let state_text = state_scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .expect("a scrollable text view holds a text view");
        state_text.setEditable(false);
        state_text.setSelectable(true);
        state_text.setRichText(false);
        state_text.setUsesFontPanel(false);
        state_text.setTextContainerInset(NSSize::new(6.0, 8.0));
        state_text.setFont(Some(&mono()));
        state_text.setAccessibilityLabel(Some(ns_string!("State")));
        state.addSubview(&state_scroll);
        let state_notice = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
        state_notice.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        state_notice.setTextColor(Some(&NSColor::secondaryLabelColor()));
        state_notice.setHidden(true);
        state_notice.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        state.addSubview(&state_notice);

        let (mailbox_scroll, mailbox) = scrolled(mtm, pane_size, "Mailbox");
        let (watches_scroll, watches) = scrolled(mtm, pane_size, "Watches");
        for (title, view) in [("State", &*state), ("Mailbox", &**mailbox_scroll), ("Watches", &**watches_scroll)] {
            let item = NSTabViewItem::new();
            item.setLabel(&NSString::from_str(title));
            item.setView(Some(view));
            tabs.addTabViewItem(&item);
        }
        let colors = [
            NSColor::controlAccentColor(),
            NSColor::systemGreenColor(),
            NSColor::systemOrangeColor(),
            NSColor::systemPurpleColor(),
            NSColor::secondaryLabelColor(),
        ];
        Self {
            root,
            table,
            footer,
            pane,
            header,
            tabs,
            state,
            state_notice,
            state_scroll,
            state_text,
            mailbox,
            mailbox_scroll,
            watches,
            watches_scroll,
            colors,
        }
    }

    /// The footer: why there is no server, or what the running status covers when that is not every running turn.
    pub(super) fn show_footer(&self, list: &ListView, target_error: Option<&str>) {
        let (footer, style) = match target_error {
            Some(error) => (error.to_owned(), Style::Error),
            None => (list.footer().unwrap_or_default(), Style::Secondary),
        };
        self.footer.setStringValue(&NSString::from_str(&footer));
        self.footer.setToolTip(Some(&NSString::from_str(&footer)));
        self.footer.setTextColor(Some(&color(style)));
    }

    /// The last root actor currently in the viewport, regardless of how many children are open.
    pub(super) fn last_visible_root(&self) -> Option<ActorPath> {
        let visible = self.table.rowsInRect(self.table.visibleRect());
        let end = visible.location.saturating_add(visible.length).min(self.table.numberOfRows() as usize);
        (visible.location..end).rev().find_map(|row| {
            let item = self.table.itemAtRow(row as NSInteger)?;
            match Item::id(&item)? {
                ItemId::Actor(path) if path.is_root() => Some(path.clone()),
                _ => None,
            }
        })
    }

    /// Fills the pane from `pane`; `state_shown` is the state the text view holds, which is set again only when it
    /// changed. Returns the state now shown.
    pub(super) fn show_pane(
        &self,
        pane: &PaneView,
        state_shown: Option<&std::sync::Arc<Json>>,
    ) -> Option<std::sync::Arc<Json>> {
        let width = self.pane.frame().size.width.max(240.0);
        let height = self.pane.frame().size.height;
        let mut layout = Layout::new(&self.header, width, PANE_MARGIN);
        header(&mut layout, pane);
        let top = layout.finish(0.0);
        self.header.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, top)));
        let empty = pane.address.is_none() || pane.not_found();
        self.tabs.setHidden(empty);
        self.tabs.setFrame(NSRect::new(NSPoint::new(0.0, top), NSSize::new(width, (height - top).max(120.0))));

        let mailbox_width = self.mailbox_scroll.contentSize().width.max(220.0);
        let mut layout = Layout::new(&self.mailbox, mailbox_width, 8.0);
        mailbox(&mut layout, pane);
        layout.finish(self.mailbox_scroll.contentSize().height);
        let watches_width = self.watches_scroll.contentSize().width.max(220.0);
        let mut layout = Layout::new(&self.watches, watches_width, 8.0);
        watches(&mut layout, pane);
        layout.finish(self.watches_scroll.contentSize().height);

        match &pane.actor {
            Read::Ready(Some(details)) => {
                if state_shown.is_some_and(|shown| std::sync::Arc::ptr_eq(shown, &details.state)) {
                    return Some(details.state.clone());
                }
                self.show_state(&details.state);
                Some(details.state.clone())
            }
            Read::Ready(None) => self.show_plain("", None),
            Read::Loading => self.show_plain(if pane.address.is_some() { "Loading…" } else { "" }, None),
            Read::Failed(error) => self.show_plain(error, Some(Style::Error)),
        }
    }

    fn show_plain(&self, text: &str, style: Option<Style>) -> Option<std::sync::Arc<Json>> {
        self.state_notice.setHidden(true);
        self.layout_state(0.0);
        self.state_text.setString(&NSString::from_str(text));
        self.state_text.setTextColor(Some(&color(style.unwrap_or(Style::Secondary))));
        None
    }

    /// The state with its runs applied as UTF-16 ranges, in one batch of the text storage.
    fn show_state(&self, json: &Json) {
        let notice = json.cut.then(|| {
            format!(
                "The state is longer than {} KiB; this shows its first {} KiB.",
                crate::json::CUT / 1024,
                crate::json::CUT / 1024
            )
        });
        self.state_notice.setHidden(notice.is_none());
        self.state_notice.setStringValue(&NSString::from_str(notice.as_deref().unwrap_or("")));
        self.layout_state(if notice.is_some() { 34.0 } else { 0.0 });
        // SAFETY: A text view made by `scrollableTextView` has a text storage.
        let Some(storage) = (unsafe { self.state_text.textStorage() }) else { return };
        let ranges = utf16_ranges(&json.text, &json.runs);
        let font = mono();
        let label = NSColor::labelColor();
        storage.beginEditing();
        storage.replaceCharactersInRange_withString(NSRange::new(0, storage.length()), &NSString::from_str(&json.text));
        let all = NSRange::new(0, storage.length());
        // SAFETY: The attribute names are AppKit's constants, the values a font and colours, and every range lies
        // within the text just set: the runs are on character boundaries of that text.
        unsafe {
            storage.addAttribute_value_range(NSFontAttributeName, as_object(&*font), all);
            storage.addAttribute_value_range(NSForegroundColorAttributeName, as_object(&*label), all);
            for (location, length, token) in ranges {
                let color = &self.colors[token_index(token)];
                storage.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    as_object(&**color),
                    NSRange::new(location, length),
                );
            }
        }
        storage.endEditing();
    }

    /// Places the state's text under a notice `notice` high.
    fn layout_state(&self, notice: f64) {
        let size = self.state.frame().size;
        let notice_frame = NSRect::new(
            NSPoint::new(8.0, size.height - notice + 4.0),
            NSSize::new(size.width - 16.0, (notice - 8.0).max(0.0)),
        );
        self.state_notice.setFrame(notice_frame);
        self.state_scroll.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(size.width, size.height - notice)));
    }

    pub(super) fn clear_actions(&self) {
        // SAFETY: Clears the table's references to the target while it is still alive.
        unsafe {
            self.table.setDataSource(None);
            self.table.setDelegate(None);
        }
        // Clear the outline's rows before the controller releases the item objects.
        self.table.reloadData();
    }
}

/// The content of a native cell, copied out of the snapshot before constructing AppKit views.
pub(super) enum CellContent {
    Actor(String),
    Status { text: String, style: Style, button: Option<Button> },
}

pub(super) enum Button {
    Retry,
    More { loading: bool },
}

impl CellContent {
    pub(super) fn of(list: &ListView, item: &ItemId, column: Option<&str>) -> Option<Self> {
        let column = column.unwrap_or("address");
        let (status, is_root) = match item {
            ItemId::Actor(path) => {
                let address = path.address();
                let text = if !path.is_root() {
                    match column {
                        "address" => address.to_string(),
                        "type" => address.type_name().to_string(),
                        "status" => list.child_status(address).to_owned(),
                        _ => String::new(),
                    }
                } else {
                    let actor = &list.rows[list.root_index(address)?];
                    match column {
                        "address" => address.to_string(),
                        "type" => actor.type_name.clone(),
                        "version" => actor.version.to_string(),
                        "created" => created(actor.created_at),
                        _ => list.status(actor).text().to_owned(),
                    }
                };
                return Some(Self::Actor(text));
            }
            ItemId::Status(None) => (root_status_row(list)?, true),
            ItemId::Status(Some(parent)) if column == "address" => (child_status_row(list, parent)?, false),
            ItemId::Status(Some(_)) => return None,
        };
        let (text, style, button) = match status {
            StatusRow::Loading => ("Loading…".into(), Style::Secondary, None),
            StatusRow::Failed(error) => {
                let text = if is_root {
                    format!("The next page failed: {error}")
                } else {
                    format!("Child page failed ({error}).")
                };
                (text, Style::Error, Some(Button::Retry))
            }
            StatusRow::NoActors if list.prefix.is_empty() => ("No actors.".into(), Style::Secondary, None),
            StatusRow::NoActors => (format!("No actors under “{}”.", list.prefix), Style::Secondary, None),
            StatusRow::NoChildren => ("No children.".into(), Style::Secondary, None),
            StatusRow::NotFound => ("Not found.".into(), Style::Error, None),
            StatusRow::More { loading } => (String::new(), Style::Secondary, Some(Button::More { loading })),
        };
        Some(Self::Status { text, style, button })
    }

    pub(super) fn view(
        self,
        mtm: MainThreadMarker,
        column: Option<&str>,
        target: &AnyObject,
        retry: Sel,
        more: Sel,
    ) -> Retained<NSView> {
        match self {
            Self::Actor(text) => {
                let field = label(mtm, &text, NSFont::systemFontOfSize(13.0), Style::Plain);
                if column == Some("address") {
                    field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
                    field.setToolTip(Some(&NSString::from_str(&text)));
                }
                if column == Some("version") {
                    field.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(13.0, 0.0)));
                }
                Retained::into_super(Retained::into_super(field))
            }
            Self::Status { text, style, button } => {
                let view =
                    NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, NSSize::new(600.0, 22.0)));
                let mut x = 0.0;
                if let Some(button) = button {
                    let (title, action, enabled) = match button {
                        Button::Retry => (ns_string!("Retry"), retry, true),
                        Button::More { loading } => (ns_string!("more…"), more, !loading),
                    };
                    // SAFETY: The target implements both actions with the button as sender, and outlives the outline.
                    let button =
                        unsafe { NSButton::buttonWithTitle_target_action(title, Some(target), Some(action), mtm) };
                    button.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(72.0, 22.0)));
                    button.setEnabled(enabled);
                    view.addSubview(&button);
                    x += 76.0;
                }
                if !text.is_empty() {
                    let field = label(mtm, &text, NSFont::systemFontOfSize(12.0), style);
                    field.setToolTip(Some(&NSString::from_str(&text)));
                    field.setFrame(NSRect::new(NSPoint::new(x, 3.0), NSSize::new(600.0 - x, 16.0)));
                    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                    view.addSubview(&field);
                }
                view
            }
        }
    }
}

/// The pane's header: the address, the actor's details and its running turn.
fn header(layout: &mut Layout<'_>, pane: &PaneView) {
    let Some(address) = &pane.address else {
        layout.text("Select an actor to see its state, running turn, mailbox and watches.", Style::Secondary);
        return;
    };
    layout.heading(address.as_str());
    match &pane.actor {
        Read::Loading => layout.text("Loading…", Style::Secondary),
        Read::Failed(error) => layout.text(error, Style::Error),
        Read::Ready(None) => {
            layout.text("Not found: this actor no longer exists.", Style::Error);
            return;
        }
        Read::Ready(Some(details)) => {
            layout.text(&format!("Type {} · version {}", details.type_name, details.version), Style::Plain);
            layout.text(&format!("Created {}", created(details.created_at)), Style::Plain);
            let parent = details.parent.as_ref().map_or("none (independent)".to_owned(), ToString::to_string);
            layout.text(&format!("Parent {parent}"), Style::Plain);
            let pinned = details.pinned_to.as_ref().map_or("any node".to_owned(), ToString::to_string);
            layout.text(&format!("Pinned to {pinned}"), Style::Plain);
            if details.paused {
                layout.text("Paused: messages queue up and no new turn starts.", Style::Plain);
            }
        }
    }
    let now = chrono::Utc::now();
    match &pane.turn {
        Read::Loading => layout.text("Turn: loading…", Style::Secondary),
        Read::Failed(error) => layout.text(&format!("Turn: {error}"), Style::Error),
        Read::Ready(None) => layout.text("Idle: no turn is running.", Style::Secondary),
        Read::Ready(Some(turn)) => {
            let message = match pane.turn_message() {
                Some(row) => format!("Running {} · attempt {}", row.message_type, row.attempt),
                None => match &pane.mailbox {
                    Read::Loading => "Running".to_owned(),
                    Read::Failed(_) => {
                        "Running (its message type is unknown: the mailbox could not be read)".to_owned()
                    }
                    Read::Ready(_) => {
                        format!("Running (its message is not among the first {MAILBOX_ROWS} of the mailbox)")
                    }
                },
            };
            layout.text(&message, Style::Plain);
            let cancel = if turn.cancel_requested { " · cancel requested" } else { "" };
            layout.text(
                &format!("{} · on {} · lease {}{cancel}", turn.message_id, turn.node, age(turn.lease_until, now)),
                Style::Secondary,
            );
        }
    }
}

/// The Mailbox tab: the pending messages among the first page; the running one is matched for the turn above.
fn mailbox(layout: &mut Layout<'_>, pane: &PaneView) {
    let mailbox = match &pane.mailbox {
        Read::Loading => return layout.text("Loading…", Style::Secondary),
        Read::Failed(error) => return layout.text(error, Style::Error),
        Read::Ready(mailbox) => mailbox,
    };
    let now = chrono::Utc::now();
    let pending: Vec<_> = pane.pending().collect();
    layout.heading(&format!("Pending ({})", pending.len()));
    if pending.is_empty() {
        layout.text("No pending messages.", Style::Secondary);
    }
    for row in pending {
        let due = format!("due {} · #{}", age(row.deliver_at, now), row.attempt);
        layout.row(&format!("{} · {}", row.message_type, row.id), &due, Some("When it is due, and its attempt"));
    }
    if mailbox.more {
        layout.text(
            &format!("The mailbox has more messages; this lists those among its first {MAILBOX_ROWS}."),
            Style::Secondary,
        );
    }
}

/// The Watches tab: where the actor watches, and where it is the exact target, apart.
fn watches(layout: &mut Layout<'_>, pane: &PaneView) {
    layout.heading("Watching");
    match &pane.watching {
        Read::Loading => layout.text("Loading…", Style::Secondary),
        Read::Failed(error) => layout.text(error, Style::Error),
        Read::Ready(watches) => {
            if watches.items.is_empty() {
                layout.text("It watches nothing.", Style::Secondary);
            }
            for watch in &watches.items {
                layout.text(watch.target.as_str(), Style::Plain);
            }
            if watches.more {
                layout.text(&format!("More watches; this lists the first {WATCH_ROWS}."), Style::Secondary);
            }
        }
    }
    layout.heading("Watched by");
    match &pane.watched_by {
        Read::Loading => layout.text("Loading…", Style::Secondary),
        Read::Failed(error) => layout.text(error, Style::Error),
        Read::Ready(watches) => {
            if watches.items.is_empty() {
                layout.text("No watch names this actor.", Style::Secondary);
            }
            for watch in &watches.items {
                layout.text(watch.watcher.as_str(), Style::Plain);
            }
            if watches.more {
                layout.text(&format!("More watchers; this lists the first {WATCH_ROWS}."), Style::Secondary);
            }
        }
    }
    layout.text(
        "Only watches on this exact actor are listed: a watch on its type or on every type (*) is not.",
        Style::Secondary,
    );
}

/// A scroll view with a flipped document, for a tab laid out from the top.
fn scrolled(mtm: MainThreadMarker, size: NSSize, name: &str) -> (Retained<NSScrollView>, Retained<FlippedView>) {
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDrawsBackground(false);
    // SAFETY: NSView's designated initializer; the class adds no ivars.
    let document: Retained<FlippedView> =
        unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, size)] };
    document.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    document.setAccessibilityLabel(Some(&NSString::from_str(name)));
    scroll.setDocumentView(Some(&document));
    (scroll, document)
}

fn mono() -> Retained<NSFont> {
    NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)
}

fn token_index(token: Token) -> usize {
    match token {
        Token::Key => 0,
        Token::String => 1,
        Token::Number => 2,
        Token::Literal => 3,
        Token::Punctuation => 4,
    }
}

fn as_object<T: AsRef<AnyObject>>(value: &T) -> &AnyObject {
    value.as_ref()
}

fn created(at: Timestamp) -> String {
    at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}
