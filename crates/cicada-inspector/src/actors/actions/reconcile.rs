//! Refreshes scheduled after writes, and immediate eviction after a delete.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use cicada::ActorAddress;

use super::{Action, ActorState, Read, Shared};
use crate::actors::{ActorPath, ListView, PaneView, PostWrite, Running};

impl Shared {
    pub(super) fn write_acknowledged(
        self: &Arc<Self>,
        state: &mut ActorState,
        action: Action,
        address: ActorAddress,
        selected: bool,
    ) {
        let pane = if action == Action::Delete {
            self.prune_deleted(state, &address);
            if selected {
                self.not_found(state, address);
            }
            None
        } else if state.selected.is_some() {
            state.invalidate_pane_read();
            Some(state.pane_generation)
        } else {
            None
        };
        state.pending_write = Some(PostWrite { pane });
        self.resume_write(state);
    }

    fn prune_deleted(self: &Arc<Self>, state: &mut ActorState, address: &ActorAddress) {
        // Fence every earlier outline read, including the later pages of a soft refresh already underway.
        state.outline_revision += 1;
        let root_page = state.pager.interrupt_page();
        let reask_root = root_page.is_some();
        state.abort(root_page);
        let sample = state.sample_task.take();
        let reask_sample = sample.is_some();
        state.abort(sample);

        let pane = self.pane.borrow();
        let gone = descendants(&self.list.borrow(), &pane, state.selected.as_ref(), address);
        drop(pane);
        let removed = |path: &ActorPath| path.0.iter().any(|address| gone.contains(address));
        self.list.send_modify(|list| {
            list.rows.retain(|row| !gone.contains(&row.address));
            list.open.retain(|path| !removed(path));
            list.children.retain(|path, page| {
                page.rows.retain(|address| !gone.contains(address));
                !removed(path)
            });
            if let Running::Sample { actors, .. } = &mut list.running {
                actors.retain(|address| !gone.contains(address));
            }
        });
        // Keep a different pane intact, even when it has no remaining occurrence in the outline.
        if state.selected.as_ref().is_some_and(removed) {
            state.selected = None;
            state.invalidate_pane_read();
        }
        state.deferred_children.retain(|path, _| !removed(path));
        state.refresh_roots = false;
        state.refresh_parents.clear();
        let mut reask_children = Vec::new();
        for (path, mut read) in std::mem::take(&mut state.child_reads) {
            let was_reading = read.task.is_some();
            state.abort(read.task.take().map(|(_, task)| task));
            if !removed(&path) {
                if was_reading {
                    reask_children.push(path.clone());
                }
                state.child_reads.insert(path, read);
            }
        }
        if reask_root {
            self.request_page(state);
        }
        if reask_sample {
            self.request_sample(state);
        }
        for path in reask_children {
            self.request_children(state, path);
        }
    }
}

/// Paths and root rows can each know about descendants that the other has never loaded.
fn descendants(
    list: &ListView,
    pane: &PaneView,
    selected: Option<&ActorPath>,
    address: &ActorAddress,
) -> BTreeSet<ActorAddress> {
    let mut edges = BTreeMap::<&ActorAddress, Vec<&ActorAddress>>::new();
    for row in &list.rows {
        if let Some(parent) = &row.parent {
            edges.entry(parent).or_default().push(&row.address);
        }
    }
    if let (Some(address), Read::Ready(Some(details))) = (&pane.address, &pane.actor)
        && let Some(parent) = &details.parent
    {
        edges.entry(parent).or_default().push(address);
    }
    for path in list.open.iter().chain(list.children.keys()).chain(selected) {
        for pair in path.0.windows(2) {
            edges.entry(&pair[0]).or_default().push(&pair[1]);
        }
    }
    for (path, page) in &list.children {
        edges.entry(path.address()).or_default().extend(&page.rows);
    }
    let mut gone = BTreeSet::new();
    let mut todo = vec![address];
    while let Some(address) = todo.pop() {
        if gone.insert(address.clone())
            && let Some(children) = edges.remove(address)
        {
            todo.extend(children);
        }
    }
    gone
}
