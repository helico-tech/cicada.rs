//! The Outcomes page: the table of finished messages with its footer and Retry, and the pane of the selected outcome or
//! linked message, with its links of the cause chain. The target owns the data and the actions; this module builds the
//! views and fills them.

use std::sync::Arc;

use cicada::store::MessageId;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSControl, NSScrollView};
use objc2_foundation::{NSInteger, NSPoint, NSRect, NSSize, ns_string};

use super::actors::created;
use super::browse::BrowseTable;
use super::content::{FlippedView, Layout, Style};
use super::json_text::JsonText;
use crate::json::Json;
use crate::model::age;
use crate::outcomes::{Caused, Details, OutcomePane, OutcomesView, Read, StatusFilter, Subject, first_line};
use crate::outcomes::{retry_text, state_text};
use crate::pager::Paging;

/// The table's columns: identifier, title and width. The status column shows only on All.
const COLUMNS: [(&str, &str, f64); 6] = [
    ("type", "Type", 120.0),
    ("actor", "Actor", 180.0),
    ("attempt", "Attempt", 80.0),
    ("finished", "Finished", 150.0),
    ("status", "Status", 90.0),
    ("error", "Error", 220.0),
];
const PANE_MARGIN: f64 = 16.0;
/// The least height the payload keeps; the details scroll in what is left above it.
const MIN_PAYLOAD: f64 = 160.0;
/// The least height of the details, also in a short pane.
const MIN_DETAILS: f64 = 120.0;
/// The tag of the Caused by link; a link of Caused has its row's index.
pub(super) const CAUSED_BY: NSInteger = -1;

/// The actions of the pane's buttons, sent to the target with the button as sender.
pub(super) struct Actions {
    /// Show Actor of the message.
    pub show_actor: Sel,
    /// A link of the cause chain, told apart by its tag: [`CAUSED_BY`] or the index of a row of Caused.
    pub follow: Sel,
    /// Retry of a failed linked read.
    pub retry_link: Sel,
    /// "more…" of Caused.
    pub more_caused: Sel,
    /// Retry of a failed page of Caused.
    pub retry_caused: Sel,
}

/// The views of the Outcomes page and its pane.
pub(super) struct OutcomesPage {
    /// The table and its footer.
    pub list: BrowseTable,
    /// The pane's root, laid out from the top.
    pub pane: Retained<FlippedView>,
    /// The details and links, in a scroll view of their own so that the payload stays in the pane.
    details: Retained<FlippedView>,
    details_scroll: Retained<NSScrollView>,
    payload: JsonText,
}

impl OutcomesPage {
    /// The page; the footer's Retry sends `retry` to `target`.
    pub(super) fn new(mtm: MainThreadMarker, target: &AnyObject, retry: Sel) -> Self {
        let list = BrowseTable::new(mtm, target, "Outcomes", &COLUMNS, retry, None);
        let pane_size = NSSize::new(320.0, 600.0);
        // SAFETY: NSView's designated initializer; the class adds no ivars.
        let pane: Retained<FlippedView> =
            unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, pane_size)] };
        pane.setAccessibilityLabel(Some(ns_string!("Outcome pane")));
        // SAFETY: as above.
        let details: Retained<FlippedView> =
            unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, pane_size)] };
        details.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let details_scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::ZERO, pane_size));
        details_scroll.setHasVerticalScroller(true);
        details_scroll.setAutohidesScrollers(true);
        details_scroll.setDrawsBackground(false);
        details_scroll.setDocumentView(Some(&details));
        details_scroll.setAccessibilityLabel(Some(ns_string!("Message details")));
        pane.addSubview(&details_scroll);
        let payload = JsonText::new(mtm, pane_size, "Payload", "payload");
        payload.root.setHidden(true);
        pane.addSubview(&payload.root);
        Self { list, pane, details, details_scroll, payload }
    }

    /// Shows the status column only on All.
    pub(super) fn show_columns(&self, status: StatusFilter) {
        if let Some(column) = self.list.table.tableColumnWithIdentifier(ns_string!("status")) {
            column.setHidden(status != StatusFilter::All);
        }
    }

    /// The footer: why there is no server; or the paging's state, with Retry after a failed page.
    pub(super) fn show_footer(&self, list: &OutcomesView, target_error: Option<&str>) {
        let what = match list.status {
            StatusFilter::All => "outcomes".to_owned(),
            status => format!("{} outcomes", status.title().to_lowercase()),
        };
        let (text, style, retry) = match (target_error, &list.paging) {
            (Some(error), _) => (error.to_owned(), Style::Error, false),
            (None, Paging::Failed(error)) => (format!("The next page failed: {error}"), Style::Error, true),
            (None, Paging::Loading) => ("Loading…".to_owned(), Style::Secondary, false),
            (None, Paging::Idle) if list.rows.is_empty() && !list.more => match list.prefix.as_str() {
                "" => (format!("No {what}."), Style::Secondary, false),
                prefix => (format!("No {what} of actors under “{prefix}”."), Style::Secondary, false),
            },
            (None, Paging::Idle) => {
                let more = if list.more { "; more load as you scroll" } else { "" };
                (format!("{} {what}{more}.", list.rows.len()), Style::Secondary, false)
            }
        };
        self.list.footer(&text, style, retry, None);
    }

    /// Fills the pane from `pane`; `payload_shown` is the payload the text view holds, which is set again only when
    /// it changed. Returns the payload now shown and the ids of the links, by tag: Caused by first, when there is one,
    /// then the rows of Caused.
    pub(super) fn show_pane(
        &self,
        pane: &OutcomePane,
        payload_shown: Option<&Arc<Json>>,
        target: &AnyObject,
        actions: &Actions,
    ) -> (Option<Arc<Json>>, Links) {
        let width = self.pane.frame().size.width.max(240.0);
        let height = self.pane.frame().size.height;
        self.clear_actions();
        let mut layout = Layout::new(&self.details, width, PANE_MARGIN);
        let links = details(&mut layout, pane, target, actions);
        let content = layout.finish(0.0);
        self.details.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, content)));
        let shown = pane.subject.as_ref().is_some_and(|subject| subject.details().is_some());
        let room = if shown { (height - MIN_PAYLOAD).max(MIN_DETAILS) } else { height };
        let top = content.min(room);
        self.details_scroll.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, top)));
        self.payload.root.setHidden(!shown);
        self.payload.root.setFrame(NSRect::new(NSPoint::new(0.0, top), NSSize::new(width, (height - top).max(60.0))));
        let payload = match &pane.payload {
            Some(payload) if payload_shown.is_some_and(|shown| Arc::ptr_eq(shown, payload)) => Some(payload.clone()),
            Some(payload) => {
                self.payload.show(payload);
                Some(payload.clone())
            }
            None => {
                self.payload.show_plain(if shown { "Formatting…" } else { "" }, None);
                None
            }
        };
        (payload, links)
    }

    /// Clears the references of the table, the footer and the pane's buttons to the target while it is still alive.
    pub(super) fn clear_actions(&self) {
        for view in self.details.subviews().to_vec() {
            if let Ok(button) = view.downcast::<NSButton>() {
                // SAFETY: Clears a reference to the target, which is still alive.
                unsafe { NSControl::setTarget(&button, None) };
            }
        }
    }

    /// Clears every reference to the target, for teardown.
    pub(super) fn teardown(&self) {
        self.clear_actions();
        self.list.clear_actions();
    }
}

/// The ids of the pane's links as they were shown: the one a button's tag names is the one followed.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Links {
    /// The message recorded as the shown one's cause (`causedBy`).
    pub caused_by: Option<MessageId>,
    /// The rows of Caused, by index.
    pub caused: Vec<MessageId>,
}

impl Links {
    /// The id the button with `tag` follows.
    pub(super) fn get(&self, tag: NSInteger) -> Option<MessageId> {
        match tag {
            CAUSED_BY => self.caused_by.clone(),
            index => self.caused.get(usize::try_from(index).ok()?).cloned(),
        }
    }
}

/// The text of a cell of the table, and its whole value for the tooltip.
pub(super) fn cell_text(list: &OutcomesView, row: NSInteger, column: &str) -> Option<(String, String)> {
    let outcome = list.rows.get(usize::try_from(row).ok()?)?;
    let text = |text: String| (text.clone(), text);
    Some(match column {
        "type" => text(outcome.message.message_type.clone()),
        "actor" => text(outcome.actor.to_string()),
        "attempt" => text(outcome.message.attempt.to_string()),
        "finished" => text(created(outcome.finished_at)),
        "status" => text(state_text(outcome.status).to_owned()),
        _ => {
            let error = outcome.error.as_deref().unwrap_or("");
            (first_line(error).to_owned(), error.to_owned())
        }
    })
}

/// The pane's details, links and buttons.
fn details(layout: &mut Layout<'_>, pane: &OutcomePane, target: &AnyObject, actions: &Actions) -> Links {
    let Some(subject) = &pane.subject else {
        layout.text("Select an outcome to see its error, envelope and payload.", Style::Secondary);
        return Links::default();
    };
    let id = subject.id();
    let details = match (subject, subject.details()) {
        (_, Some(details)) => details,
        (Subject::Linked(_, Read::Failed(error)), None) => {
            layout.heading(&format!("Message {id}"));
            layout.text(&format!("The read failed: {error}"), Style::Error);
            layout.button("Retry", target, actions.retry_link, 0, &format!("Read message {id} again"));
            return Links::default();
        }
        (Subject::Linked(_, Read::NotFound), None) => {
            layout.heading(&format!("Message {id}"));
            layout.text("Not found: the message is unknown, or past retention.", Style::Secondary);
            return Links::default();
        }
        _ => {
            layout.heading(&format!("Message {id}"));
            layout.text("Loading…", Style::Secondary);
            return Links::default();
        }
    };
    envelope(layout, id, &details);
    let to = details.message.to.to_string();
    layout.button("Show Actor", target, actions.show_actor, 0, &format!("Show Actor {to}"));
    let mut links = Links::default();
    layout.heading("Caused by");
    match &details.message.caused_by {
        Some(cause) => {
            let text = format!("Message {cause}");
            layout.link(&text, target, actions.follow, CAUSED_BY, &format!("Show the recorded cause, message {cause}"));
            links.caused_by = Some(cause.clone());
        }
        None => layout.text("No recorded cause", Style::Secondary),
    }
    layout.heading("Caused");
    links.caused = caused(layout, &pane.caused, target, actions);
    links
}

/// The status, error, attempts, retry policy and envelope.
fn envelope(layout: &mut Layout<'_>, id: &MessageId, details: &Details<'_>) {
    let message = details.message;
    layout.heading(&message.message_type);
    let status = state_text(details.status);
    match (details.node, details.lease_until) {
        (Some(node), Some(lease)) => {
            layout.text(&format!("Status {status} on {node}, lease {}", age(lease, chrono::Utc::now())), Style::Plain)
        }
        _ => layout.text(&format!("Status {status}"), Style::Plain),
    }
    if let Some(error) = details.error {
        layout.text(error, Style::Error);
    }
    layout.text(&format!("Attempt {} · failures {}", message.attempt, message.failures), Style::Plain);
    if let Some(error) = &message.last_error {
        layout.text(&format!("Last retried error: {error}"), Style::Secondary);
    }
    layout.text(&retry_text(message.retry.as_ref()), Style::Plain);
    layout.text(&format!("Id {id}"), Style::Plain);
    layout.text(&format!("To {}", message.to), Style::Plain);
    let from = message.from.as_ref().map_or("outside an actor".to_owned(), ToString::to_string);
    layout.text(&format!("From {from}"), Style::Plain);
    match &message.correlation_id {
        Some(id) => layout.text(&format!("Correlation id {id}"), Style::Plain),
        None => layout.text("No correlation id", Style::Secondary),
    }
    match details.finished_at {
        Some(at) => layout.text(&format!("Finished {}", created(at)), Style::Plain),
        None => layout.text(&format!("Deliver at {}", created(message.deliver_at)), Style::Plain),
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

/// The rows of Caused as links, and its state: "Loading…", the error with Retry, "more…", "None" or "not found".
fn caused(layout: &mut Layout<'_>, caused: &Caused, target: &AnyObject, actions: &Actions) -> Vec<MessageId> {
    let mut ids = Vec::with_capacity(caused.rows.len());
    for (index, row) in caused.rows.iter().enumerate() {
        let text = format!("{} · {} · {}", row.message.message_type, state_text(row.status), row.id);
        let tag = NSInteger::try_from(index).unwrap_or(NSInteger::MAX);
        layout.link(&text, target, actions.follow, tag, &format!("Show the caused message {}", row.id));
        ids.push(row.id.clone());
    }
    if caused.gone {
        layout.text("Not found: the message is past retention.", Style::Secondary);
    } else if caused.none() {
        layout.text("None", Style::Secondary);
    }
    match &caused.paging {
        Paging::Loading => layout.text("Loading…", Style::Secondary),
        Paging::Failed(error) => {
            layout.text(&format!("Caused could not be read: {error}"), Style::Error);
            layout.button("Retry", target, actions.retry_caused, 0, "Read the caused messages again");
        }
        Paging::Idle if caused.more => {
            layout.button("more…", target, actions.more_caused, 0, "Read more caused messages");
        }
        Paging::Idle => {}
    }
    ids
}
