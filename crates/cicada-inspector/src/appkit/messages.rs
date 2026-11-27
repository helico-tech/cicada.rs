//! The Messages page: the table of pending messages with its footer and Retry, and the pane of the selected message
//! with Show Actor. The target owns the data and the actions; this module builds the views and fills them.

use std::sync::Arc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSControl, NSFont, NSLineBreakMode, NSScrollView,
    NSTableColumn, NSTableColumnResizingOptions, NSTableView, NSTableViewColumnAutoresizingStyle, NSTableViewStyle,
    NSTextField, NSView,
};
use objc2_foundation::{NSInteger, NSPoint, NSRect, NSSize, NSString, ns_string};

use super::actors::created;
use super::content::{FlippedView, Layout, Style, color, label};
use super::json_text::JsonText;
use crate::json::Json;
use crate::messages::{MessagePane, MessagesView};
use crate::pager::Paging;

/// The table's columns: identifier, title and width.
const COLUMNS: [(&str, &str, f64); 5] = [
    ("id", "Id", 220.0),
    ("to", "To", 200.0),
    ("type", "Type", 120.0),
    ("deliver", "Deliver at", 150.0),
    ("attempt", "Attempt", 70.0),
];
const FOOTER: f64 = 26.0;
const PANE_MARGIN: f64 = 16.0;
const BUTTON: NSSize = NSSize::new(110.0, 24.0);
/// The least height the payload keeps under Show Actor; the envelope scrolls in what is left above it.
const MIN_PAYLOAD: f64 = 160.0;
/// The least height of the envelope, also in a short pane.
const MIN_ENVELOPE: f64 = 80.0;

/// The views of the Messages page and its pane.
pub(super) struct MessagesPage {
    /// The table with the footer under it.
    pub root: Retained<NSView>,
    pub table: Retained<NSTableView>,
    footer: Retained<NSTextField>,
    retry: Retained<NSButton>,
    /// The pane's root, laid out from the top.
    pub pane: Retained<FlippedView>,
    /// The envelope, in a scroll view of its own so that Show Actor and the payload stay in the pane.
    header: Retained<FlippedView>,
    header_scroll: Retained<NSScrollView>,
    show_actor: Retained<NSButton>,
    payload: JsonText,
}

impl MessagesPage {
    /// The page; Retry and Show Actor send `retry` and `show_actor` to `target`.
    pub(super) fn new(mtm: MainThreadMarker, target: &AnyObject, retry: Sel, show_actor: Sel) -> Self {
        let size = NSSize::new(760.0, 600.0);
        let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
        let table = NSTableView::new(mtm);
        for (identifier, title, width) in COLUMNS {
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str(identifier));
            column.setTitle(&NSString::from_str(title));
            column.setWidth(width);
            column.setMinWidth(50.0);
            column.setResizingMask(NSTableColumnResizingOptions::UserResizingMask);
            table.addTableColumn(&column);
        }
        table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle);
        table.setStyle(NSTableViewStyle::Inset);
        table.setUsesAlternatingRowBackgroundColors(true);
        table.setAllowsEmptySelection(true);
        table.setRowHeight(22.0);
        table.setAccessibilityLabel(Some(ns_string!("Pending messages")));
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
        // SAFETY: The target implements the action with the button as the sender, and clears it during teardown.
        let retry =
            unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Retry"), Some(target), Some(retry), mtm) };
        retry.setFrame(NSRect::new(NSPoint::new(8.0, 2.0), NSSize::new(72.0, 22.0)));
        retry.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMaxYMargin);
        retry.setHidden(true);
        root.addSubview(&retry);
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
        pane.setAccessibilityLabel(Some(ns_string!("Message pane")));
        // SAFETY: as above.
        let header: Retained<FlippedView> =
            unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, pane_size)] };
        header.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let header_scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::ZERO, pane_size));
        header_scroll.setHasVerticalScroller(true);
        header_scroll.setAutohidesScrollers(true);
        header_scroll.setDrawsBackground(false);
        header_scroll.setDocumentView(Some(&header));
        header_scroll.setAccessibilityLabel(Some(ns_string!("Envelope")));
        pane.addSubview(&header_scroll);
        // SAFETY: The target implements the action with the button as the sender, and clears it during teardown.
        let show_actor = unsafe {
            NSButton::buttonWithTitle_target_action(ns_string!("Show Actor"), Some(target), Some(show_actor), mtm)
        };
        show_actor.setToolTip(Some(ns_string!("Select the message's actor on the Actors page")));
        show_actor.setHidden(true);
        pane.addSubview(&show_actor);
        let payload = JsonText::new(mtm, pane_size, "Payload", "payload");
        payload.root.setHidden(true);
        pane.addSubview(&payload.root);
        Self { root, table, footer, retry, pane, header, header_scroll, show_actor, payload }
    }

    /// The footer: why there is no server; or the paging's state, with Retry after a failed page.
    pub(super) fn show_footer(&self, list: &MessagesView, target_error: Option<&str>) {
        let (text, style, failed) = match (target_error, &list.paging) {
            (Some(error), _) => (error.to_owned(), Style::Error, false),
            (None, Paging::Failed(error)) => (format!("The next page failed: {error}"), Style::Error, true),
            (None, Paging::Loading) => ("Loading…".to_owned(), Style::Secondary, false),
            (None, Paging::Idle) if list.rows.is_empty() && !list.more => match list.prefix.as_str() {
                "" => ("No pending messages.".to_owned(), Style::Secondary, false),
                prefix => (format!("No pending messages to actors under “{prefix}”."), Style::Secondary, false),
            },
            (None, Paging::Idle) => {
                let more = if list.more { "; more load as you scroll" } else { "" };
                (format!("{} pending messages{more}.", list.rows.len()), Style::Secondary, false)
            }
        };
        let (text, tip) = super::browse::freshness(text, list.stale.as_ref());
        self.retry.setHidden(!failed);
        let x = if failed { 88.0 } else { 12.0 };
        let width = self.root.frame().size.width;
        self.footer.setFrame(NSRect::new(NSPoint::new(x, 5.0), NSSize::new((width - x - 12.0).max(40.0), 16.0)));
        self.footer.setStringValue(&NSString::from_str(&text));
        self.footer.setToolTip(Some(&NSString::from_str(&tip)));
        self.footer.setTextColor(Some(&color(style)));
    }

    /// The last row in the viewport.
    pub(super) fn last_visible_row(&self) -> Option<usize> {
        let visible = self.table.rowsInRect(self.table.visibleRect());
        let end = visible.location.saturating_add(visible.length).min(self.table.numberOfRows() as usize);
        end.checked_sub(1)
    }

    /// Fills the pane from `pane`; `payload_shown` is the payload the text view holds, which is set again only when
    /// it changed. Returns the payload now shown.
    pub(super) fn show_pane(&self, pane: &MessagePane, payload_shown: Option<&Arc<Json>>) -> Option<Arc<Json>> {
        let width = self.pane.frame().size.width.max(240.0);
        let height = self.pane.frame().size.height;
        let mut layout = Layout::new(&self.header, width, PANE_MARGIN);
        header(&mut layout, pane);
        let content = layout.finish(0.0);
        self.header.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, content)));
        let shown = pane.message.is_some();
        // Show Actor and the payload keep their place; a long envelope scrolls above them.
        let room = if shown { (height - BUTTON.height - 12.0 - MIN_PAYLOAD).max(MIN_ENVELOPE) } else { height };
        let top = content.min(room);
        self.header_scroll.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, top)));
        self.show_actor.setHidden(!shown);
        self.payload.root.setHidden(!shown);
        self.show_actor.setFrame(NSRect::new(NSPoint::new(PANE_MARGIN + 8.0, top), BUTTON));
        let below = top + BUTTON.height + 12.0;
        self.payload
            .root
            .setFrame(NSRect::new(NSPoint::new(0.0, below), NSSize::new(width, (height - below).max(60.0))));
        if let Some(message) = &pane.message {
            let to = NSString::from_str(&format!("Show Actor {}", message.to));
            self.show_actor.setAccessibilityLabel(Some(&to));
        }
        match &pane.payload {
            Some(payload) if payload_shown.is_some_and(|shown| Arc::ptr_eq(shown, payload)) => Some(payload.clone()),
            Some(payload) => {
                self.payload.show(payload);
                Some(payload.clone())
            }
            None => {
                self.payload.show_plain(if shown { "Formatting…" } else { "" }, None);
                None
            }
        }
    }

    /// Clears the table's and the buttons' references to the target while it is still alive.
    pub(super) fn clear_actions(&self) {
        // SAFETY: Clears references to the target, which is still alive.
        unsafe {
            self.table.setDataSource(None);
            self.table.setDelegate(None);
            NSControl::setTarget(&self.retry, None);
            NSControl::setTarget(&self.show_actor, None);
        }
        self.table.reloadData();
    }
}

/// The text of a cell of the table.
pub(super) fn cell_text(list: &MessagesView, row: NSInteger, column: &str) -> Option<String> {
    let message = list.rows.get(usize::try_from(row).ok()?)?;
    Some(match column {
        "id" => message.id.to_string(),
        "to" => message.to.to_string(),
        "type" => message.message_type.clone(),
        "deliver" => created(message.deliver_at),
        _ => message.attempt.to_string(),
    })
}

/// A cell of the table.
pub(super) fn cell(mtm: MainThreadMarker, text: &str, column: &str) -> Retained<NSView> {
    let field = label(mtm, text, NSFont::systemFontOfSize(13.0), Style::Plain);
    match column {
        "id" | "to" => {
            field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
            field.setToolTip(Some(&NSString::from_str(text)));
        }
        "attempt" => field.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(13.0, 0.0))),
        _ => {}
    }
    Retained::into_super(Retained::into_super(field))
}

/// The pane's header: the message's envelope.
fn header(layout: &mut Layout<'_>, pane: &MessagePane) {
    let Some(message) = &pane.message else {
        layout.text("Select a message to see its envelope and payload.", Style::Secondary);
        return;
    };
    layout.heading(&message.message_type);
    layout.text(&format!("Id {}", message.id), Style::Plain);
    layout.text(&format!("To {}", message.to), Style::Plain);
    let from = message.from.as_ref().map_or("outside an actor".to_owned(), ToString::to_string);
    layout.text(&format!("From {from}"), Style::Plain);
    layout.text(&format!("Deliver at {}", created(message.deliver_at)), Style::Plain);
    layout.text(&format!("Attempt {} · failures {}", message.attempt, message.failures), Style::Plain);
    match &message.correlation_id {
        Some(id) => layout.text(&format!("Correlation id {id}"), Style::Plain),
        None => layout.text("No correlation id", Style::Secondary),
    }
    if message.headers.is_empty() {
        layout.text("No headers", Style::Secondary);
    } else {
        layout.text("Headers", Style::Secondary);
        for (name, value) in &message.headers {
            layout.text(&format!("{name}: {value}"), Style::Plain);
        }
    }
}
