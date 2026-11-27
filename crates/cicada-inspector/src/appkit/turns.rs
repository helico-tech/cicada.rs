//! The Running Turns page: the table of running turns with its footer and Retry, and the pane of the selected turn
//! with Show Actor. The target owns the data and the actions; this module builds the views and fills them.

use objc2::MainThreadMarker;
use objc2::runtime::{AnyObject, Sel};
use objc2_foundation::NSInteger;

use super::actors::created;
use super::browse::{BrowseTable, DetailPane};
use super::content::Style;
use crate::pager::Paging;
use crate::turns::{TurnPane, TurnState, TurnsView};

/// The table's columns: identifier, title and width.
const COLUMNS: [(&str, &str, f64); 5] = [
    ("actor", "Actor", 200.0),
    ("message", "Message", 220.0),
    ("node", "Node", 120.0),
    ("lease", "Lease until", 160.0),
    ("cancel", "Cancel", 100.0),
];

/// The actions of the pane's buttons, sent to the target with the button as sender.
pub(super) struct Actions {
    /// Show Actor of the turn.
    pub show_actor: Sel,
    /// Retry of a failed read of the turn on a return.
    pub retry_check: Sel,
}

/// The views of the Running Turns page and its pane.
pub(super) struct TurnsPage {
    /// The table and its footer.
    pub list: BrowseTable,
    /// The pane of the selected turn.
    pub pane: DetailPane,
}

impl TurnsPage {
    /// The page; the footer's Retry sends `retry` to `target`.
    pub(super) fn new(mtm: MainThreadMarker, target: &AnyObject, retry: Sel) -> Self {
        let list = BrowseTable::new(mtm, target, "Running turns", &COLUMNS, retry, None);
        Self { list, pane: DetailPane::new(mtm, "Turn pane") }
    }

    /// The footer: why there is no server; or the paging's state, with Retry after a failed page.
    pub(super) fn show_footer(&self, list: &TurnsView, target_error: Option<&str>) {
        let (text, style, retry) = match (target_error, &list.paging) {
            (Some(error), _) => (error.to_owned(), Style::Error, false),
            (None, Paging::Failed(error)) => (format!("The next page failed: {error}"), Style::Error, true),
            (None, Paging::Loading) => ("Loading…".to_owned(), Style::Secondary, false),
            (None, Paging::Idle) if list.rows.is_empty() && !list.more => match list.prefix.as_str() {
                "" => ("No turns are running.".to_owned(), Style::Secondary, false),
                prefix => (format!("No turns are running for actors under “{prefix}”."), Style::Secondary, false),
            },
            (None, Paging::Idle) => {
                let more = if list.more { "; more load as you scroll" } else { "" };
                (format!("{} running turns{more}.", list.rows.len()), Style::Secondary, false)
            }
        };
        self.list.footer(&text, style, retry, None);
    }

    /// Fills the pane from `pane`.
    pub(super) fn show_pane(&self, pane: &TurnPane, target: &AnyObject, actions: &Actions) {
        self.pane.show(|layout| {
            let Some(turn) = &pane.turn else {
                layout.text("Select a turn to see its message, node and lease.", Style::Secondary);
                return;
            };
            layout.heading(turn.actor.as_str());
            match &pane.state {
                TurnState::Running => layout.text("Running", Style::Plain),
                TurnState::Checking => layout.text("Checking whether it still runs…", Style::Secondary),
                TurnState::Ended => layout.text("No longer running: this is the turn as it was read.", Style::Error),
                TurnState::Failed(error) => {
                    layout.text(&format!("Could not check whether it still runs: {error}"), Style::Error);
                    let label = format!("Read the running turn of {} again", turn.actor);
                    layout.button("Retry", target, actions.retry_check, 0, &label);
                }
            }
            layout.text(&format!("Message {}", turn.message_id), Style::Plain);
            layout.text(&format!("Node {}", turn.node), Style::Plain);
            layout.text(&format!("Lease until {}", created(turn.lease_until)), Style::Plain);
            if turn.cancel_requested {
                layout.text("An operator asked to cancel this turn.", Style::Secondary);
            }
            let label = format!("Show Actor {}", turn.actor);
            layout.button("Show Actor", target, actions.show_actor, 0, &label);
        });
    }

    /// Clears every reference to the target, for teardown.
    pub(super) fn teardown(&self) {
        self.pane.clear_actions();
        self.list.clear_actions();
    }
}

/// The text of a cell of the table.
pub(super) fn cell_text(list: &TurnsView, row: NSInteger, column: &str) -> Option<String> {
    let turn = list.rows.get(usize::try_from(row).ok()?)?;
    Some(match column {
        "actor" => turn.actor.to_string(),
        "message" => turn.message_id.to_string(),
        "node" => turn.node.to_string(),
        "lease" => created(turn.lease_until),
        _ => (if turn.cancel_requested { "asked" } else { "" }).to_owned(),
    })
}
