//! The Watches table and the selected watch's two endpoints.

use cicada::WatchTarget;
use cicada::api::WatchView;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{NSAccessibility, NSButton, NSControl, NSFont, NSLineBreakMode, NSTextField};
use objc2_foundation::{NSInteger, NSPoint, NSRect, NSSize, NSString, ns_string};

use super::browse::BrowseTable;
use super::content::{FlippedView, Style, label};
use crate::pager::Paging;
use crate::watches::WatchesView;

/// The table and inspector pane for the Watches page.
pub(super) struct WatchesPage {
    /// The table and its footer.
    pub list: BrowseTable,
    /// The pane of the selected watch.
    pub pane: Retained<FlippedView>,
    target: Retained<NSTextField>,
    watcher: Retained<NSTextField>,
    show_target: Retained<NSButton>,
    show_watcher: Retained<NSButton>,
}

impl WatchesPage {
    /// The page; the two Show Actor buttons send their own selectors to `target`.
    pub(super) fn new(
        mtm: MainThreadMarker,
        target: &AnyObject,
        retry: Sel,
        more: Sel,
        show_target: Sel,
        show_watcher: Sel,
    ) -> Self {
        let list = BrowseTable::new(
            mtm,
            target,
            "Stored watches",
            &[("target", "Target", 210.0), ("watcher", "Watcher", 210.0)],
            retry,
            Some(more),
        );
        let rect = NSRect::new(NSPoint::ZERO, NSSize::new(320.0, 600.0));
        // SAFETY: NSView's designated initializer; the class adds no ivars.
        let pane: Retained<FlippedView> = unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: rect] };
        pane.setAccessibilityLabel(Some(ns_string!("Watch pane")));
        let endpoint = || {
            let field = label(mtm, "", NSFont::systemFontOfSize(13.0), Style::Plain);
            field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
            field.setSelectable(true);
            pane.addSubview(&field);
            field
        };
        let button = |action| {
            // SAFETY: The target implements the action with the button as sender and clears it on teardown.
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(ns_string!("Show Actor"), Some(target), Some(action), mtm)
            };
            pane.addSubview(&button);
            button
        };
        let (target, watcher) = (endpoint(), endpoint());
        let (show_target, show_watcher) = (button(show_target), button(show_watcher));
        Self { list, pane, target, watcher, show_target, show_watcher }
    }

    /// Shows read status and available footer actions.
    pub(super) fn show_footer(&self, view: &WatchesView, target_error: Option<&str>) {
        let filtered = !view.prefix.is_empty();
        let (text, style, retry, more) = match (target_error, &view.paging) {
            (Some(error), _) => (error.to_owned(), Style::Error, false, None),
            (None, Paging::Failed(error)) => (format!("Could not load watches: {error}"), Style::Error, true, None),
            (None, Paging::Loading) => {
                ("Loading watches…".to_owned(), Style::Secondary, false, view.more.then_some(false))
            }
            (None, Paging::Idle) => {
                let text = if filtered {
                    format!("{} of {} watches match.", view.rows.len(), view.loaded)
                } else if view.loaded == 0 {
                    "No stored watches.".to_owned()
                } else {
                    let next = if view.more { "; more load as you scroll" } else { "" };
                    format!("{} watches{next}.", view.loaded)
                };
                (text, Style::Secondary, false, view.more.then_some(true))
            }
        };
        let text = if filtered && target_error.is_none() { format!("Filter covers loaded rows. {text}") } else { text };
        let (text, tip) = super::browse::freshness(text, view.stale.as_ref());
        self.list.footer(&text, style, retry, more);
        self.list.footer_tip(&tip);
    }

    /// Shows the selected watch in the pane.
    pub(super) fn show_pane(&self, selected: Option<&WatchView>) {
        let width = (self.pane.frame().size.width - 32.0).max(40.0);
        let actor_target = selected.is_some_and(|watch| matches!(watch.target, WatchTarget::Actor(_)));
        let watcher_y = if actor_target { 82.0 } else { 54.0 };
        self.target.setFrame(NSRect::new(NSPoint::new(16.0, 16.0), NSSize::new(width, 22.0)));
        self.watcher.setFrame(NSRect::new(NSPoint::new(16.0, watcher_y), NSSize::new(width, 22.0)));
        self.show_target.setFrame(NSRect::new(NSPoint::new(16.0, 40.0), NSSize::new(112.0, 24.0)));
        self.show_watcher.setFrame(NSRect::new(NSPoint::new(16.0, watcher_y + 24.0), NSSize::new(112.0, 24.0)));
        self.watcher.setHidden(selected.is_none());
        self.show_watcher.setHidden(selected.is_none());
        self.show_target.setHidden(!actor_target);
        if let Some(watch) = selected {
            set_endpoint(&self.target, &format!("Target {}", watch.target));
            set_endpoint(&self.watcher, &format!("Watcher {}", watch.watcher));
            let target = NSString::from_str(&format!("Show Actor for target {}", watch.target));
            let watcher = NSString::from_str(&format!("Show Actor for watcher {}", watch.watcher));
            self.show_target.setAccessibilityLabel(Some(&target));
            self.show_target.setToolTip(Some(&target));
            self.show_watcher.setAccessibilityLabel(Some(&watcher));
            self.show_watcher.setToolTip(Some(&watcher));
        } else {
            set_endpoint(&self.target, "Select a watch to see its target and watcher.");
        }
    }

    /// Detaches selectors before the target is released.
    pub(super) fn clear_actions(&self) {
        self.list.clear_actions();
        // SAFETY: Both buttons accept a nil target, and the target is still alive.
        unsafe {
            NSControl::setTarget(&self.show_target, None);
            NSControl::setTarget(&self.show_watcher, None);
        }
    }
}

/// The text for a cell at this row and column.
pub(super) fn cell_text(view: &WatchesView, row: NSInteger, column: &str) -> Option<String> {
    let watch = view.rows.get(usize::try_from(row).ok()?)?;
    Some(if column == "target" { watch.target.to_string() } else { watch.watcher.to_string() })
}

fn set_endpoint(field: &NSTextField, text: &str) {
    let text = NSString::from_str(text);
    field.setStringValue(&text);
    field.setToolTip(Some(&text));
}
