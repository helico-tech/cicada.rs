//! The Nodes page: the table of nodes with its footer and Retry, and the pane of the selected node with its running
//! turns, each with Show Actor. The target owns the data and the actions; this module builds the views and fills them.

use cicada::ActorAddress;
use objc2::MainThreadMarker;
use objc2::runtime::{AnyObject, Sel};
use objc2_foundation::NSInteger;

use super::actors::created;
use super::browse::{BrowseTable, DetailPane};
use super::content::Style;
use crate::nodes::{
    AS_OF, NodePane, NodeRead, NodesRead, NodesView, column_text, health_text, hosted_text, precise_time,
};

/// The table's columns: identifier, title and width.
const COLUMNS: [(&str, &str, f64); 7] = [
    ("id", "Id", 140.0),
    ("status", "Status", 80.0),
    ("hosted", "Hosts", 160.0),
    ("label", "Label", 100.0),
    ("host", "Host", 120.0),
    ("seen", "Last heartbeat", 160.0),
    ("turns", "Running", 80.0),
];

/// The actions of the pane's buttons, sent to the target with the button as sender.
pub(super) struct Actions {
    /// Show Actor of a turn of the node, told apart by its tag: the turn's index.
    pub show_actor: Sel,
    /// Retry of a failed read of the node.
    pub retry_node: Sel,
}

/// The views of the Nodes page and its pane.
pub(super) struct NodesPage {
    /// The table and its footer.
    pub list: BrowseTable,
    /// The pane of the selected node.
    pub pane: DetailPane,
}

impl NodesPage {
    /// The page; the footer's Retry sends `retry` to `target`.
    pub(super) fn new(mtm: MainThreadMarker, target: &AnyObject, retry: Sel) -> Self {
        let list = BrowseTable::new(mtm, target, "Nodes", &COLUMNS, retry, None);
        Self { list, pane: DetailPane::new(mtm, "Node pane") }
    }

    /// The footer: why there is no server; or the read's state, with Retry after a failed read. Its tooltip gives the
    /// time of the last read on this Mac's clock.
    pub(super) fn show_footer(&self, list: &NodesView, target_error: Option<&str>) {
        let (text, style, retry) = match (target_error, &list.read) {
            (Some(error), _) => (error.to_owned(), Style::Error, false),
            (None, NodesRead::Failed(error)) => (format!("The nodes could not be read: {error}"), Style::Error, true),
            (None, NodesRead::Loading | NodesRead::NotRead) => ("Loading…".to_owned(), Style::Secondary, false),
            (None, NodesRead::Ready(_)) if list.loaded == 0 => ("No nodes.".to_owned(), Style::Secondary, false),
            (None, NodesRead::Ready(_)) if !list.prefix.is_empty() => {
                let text = format!("{} of {} nodes match; status {AS_OF}.", list.rows.len(), list.loaded);
                (text, Style::Secondary, false)
            }
            (None, NodesRead::Ready(_)) => (format!("{} nodes; status {AS_OF}.", list.loaded), Style::Secondary, false),
        };
        self.list.footer(&text, style, retry, None);
        if let NodesRead::Ready(at) = &list.read {
            self.list.footer_tip(&format!("{text} Read at {} on this Mac's clock.", created(*at)));
        }
    }

    /// Fills the pane from `pane`; returns the actors of the turns shown, by the tag of their Show Actor button.
    pub(super) fn show_pane(&self, pane: &NodePane, target: &AnyObject, actions: &Actions) -> Vec<ActorAddress> {
        let mut actors = Vec::new();
        self.pane.show(|layout| {
            let Some(id) = &pane.node else {
                layout.text("Select a node to see its running turns.", Style::Secondary);
                return;
            };
            layout.heading(id.as_str());
            let detail = match &pane.read {
                NodeRead::Loading => return layout.text("Loading…", Style::Secondary),
                NodeRead::NotFound => return layout.text("Not found: the node's row was removed.", Style::Secondary),
                NodeRead::Failed(error) => {
                    layout.text(&format!("The node could not be read: {error}"), Style::Error);
                    layout.button("Retry", target, actions.retry_node, 0, &format!("Read node {id} again"));
                    return;
                }
                NodeRead::Found(detail) => detail,
            };
            let node = &detail.node;
            layout.text(&format!("Status {}, {AS_OF}", health_text(node.status)), Style::Plain);
            layout.text(&format!("Last heartbeat {}", precise_time(node.last_seen)), Style::Plain);
            layout.text(&format!("Started {}", precise_time(node.started_at)), Style::Plain);
            layout.text(&format!("Hosts {}", hosted_text(&node.hosted)), Style::Plain);
            for (name, value) in [("Label", &node.label), ("Host", &node.hostname), ("Cicada", &node.cicada_version)] {
                if let Some(value) = value {
                    layout.text(&format!("{name} {value}"), Style::Plain);
                }
            }
            layout.heading(&format!("Running turns ({})", detail.turns.len()));
            if detail.turns.is_empty() {
                layout.text("None", Style::Secondary);
            }
            for (index, turn) in detail.turns.iter().enumerate() {
                let cancel = if turn.cancel_requested { " · cancel asked" } else { "" };
                let lease = created(turn.lease_until);
                layout.text(
                    &format!("{} · message {} · lease until {lease}{cancel}", turn.actor, turn.message_id),
                    Style::Plain,
                );
                let tag = NSInteger::try_from(index).unwrap_or(NSInteger::MAX);
                layout.button("Show Actor", target, actions.show_actor, tag, &format!("Show Actor {}", turn.actor));
                actors.push(turn.actor.clone());
            }
        });
        actors
    }

    /// Clears every reference to the target, for teardown.
    pub(super) fn teardown(&self) {
        self.pane.clear_actions();
        self.list.clear_actions();
    }
}

/// The text of a cell of the table.
pub(super) fn cell_text(list: &NodesView, row: NSInteger, column: &str) -> Option<String> {
    let node = list.rows.get(usize::try_from(row).ok()?)?;
    Some(column_text(node, column))
}
