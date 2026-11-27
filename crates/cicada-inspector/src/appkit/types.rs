//! The Types table and its schema chooser and JSON view.

use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{NSAccessibility, NSControl, NSFont, NSLineBreakMode, NSPopUpButton, NSTextField};
use objc2_foundation::{NSInteger, NSPoint, NSRect, NSSize, NSString, ns_string};

use super::browse::BrowseTable;
use super::content::{FlippedView, Style, label};
use super::json_text::JsonText;
use crate::pager::Paging;
use crate::types::{TypePane, TypesView};

/// The table and inspector pane for the Types page.
pub(super) struct TypesPage {
    /// The table and its footer.
    pub list: BrowseTable,
    /// The pane of the selected type.
    pub pane: Retained<FlippedView>,
    title: Retained<NSTextField>,
    chooser: Retained<NSPopUpButton>,
    schema: JsonText,
}

impl TypesPage {
    /// The page; the schema chooser sends `choose` with itself as sender.
    pub(super) fn new(mtm: MainThreadMarker, target: &AnyObject, retry: Sel, choose: Sel) -> Self {
        let list = BrowseTable::new(
            mtm,
            target,
            "Registered types",
            &[("name", "Name", 170.0), ("hash", "Contract hash", 150.0), ("messages", "Message types", 110.0)],
            retry,
            None,
        );
        let size = NSSize::new(320.0, 600.0);
        let rect = NSRect::new(NSPoint::ZERO, size);
        // SAFETY: NSView's designated initializer; the class adds no ivars.
        let pane: Retained<FlippedView> = unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: rect] };
        pane.setAccessibilityLabel(Some(ns_string!("Type pane")));
        let title = label(mtm, "", NSFont::boldSystemFontOfSize(15.0), Style::Plain);
        title.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        pane.addSubview(&title);
        let chooser = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect, false);
        // SAFETY: The target implements the action with the pop-up as sender and clears it on teardown.
        unsafe {
            NSControl::setTarget(&chooser, Some(target));
            NSControl::setAction(&chooser, Some(choose));
        }
        chooser.setAccessibilityLabel(Some(ns_string!("Schema")));
        pane.addSubview(&chooser);
        let schema = JsonText::new(mtm, size, "JSON Schema", "schema");
        pane.addSubview(&schema.root);
        Self { list, pane, title, chooser, schema }
    }

    /// Shows read status and available footer actions.
    pub(super) fn show_footer(&self, view: &TypesView, target_error: Option<&str>) {
        let (text, style, retry) = match (target_error, &view.paging) {
            (Some(error), _) => (error.to_owned(), Style::Error, false),
            (None, Paging::Failed(error)) => (format!("Could not load types: {error}"), Style::Error, true),
            (None, Paging::Loading) => ("Loading types…".to_owned(), Style::Secondary, false),
            (None, Paging::Idle) if view.rows.is_empty() => {
                let text = if view.prefix.is_empty() { "No registered types." } else { "No matching types." };
                (text.to_owned(), Style::Secondary, false)
            }
            (None, Paging::Idle) => (format!("{} registered types.", view.rows.len()), Style::Secondary, false),
        };
        let (text, tip) = super::browse::freshness(text, view.stale.as_ref());
        self.list.footer(&text, style, retry, None);
        self.list.footer_tip(&tip);
    }

    /// Shows the selected type in the pane.
    pub(super) fn show_pane(&self, pane: &TypePane, previous: &TypePane) {
        let size = self.pane.frame().size;
        let width = (size.width - 32.0).max(40.0);
        self.title.setFrame(NSRect::new(NSPoint::new(16.0, 16.0), NSSize::new(width, 22.0)));
        self.chooser.setFrame(NSRect::new(NSPoint::new(16.0, 44.0), NSSize::new(width, 24.0)));
        self.schema
            .root
            .setFrame(NSRect::new(NSPoint::new(0.0, 76.0), NSSize::new(size.width, (size.height - 76.0).max(60.0))));
        self.chooser.setHidden(pane.row.is_none());
        self.schema.root.setHidden(pane.row.is_none());
        let title = NSString::from_str(pane.row.as_ref().map_or("Select a type to see its schemas.", |row| &row.name));
        self.title.setStringValue(&title);
        self.title.setToolTip(Some(&title));
        let changed = match (&pane.row, &previous.row) {
            (Some(row), Some(before)) => !Arc::ptr_eq(row, before),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.chooser.removeAllItems();
            let names = pane.schemas();
            for (index, name) in names.iter().enumerate() {
                let mut title = (*name).to_owned();
                // AppKit replaces an item with the same title. Give a message named State its own title.
                if index != 0 && *name == "State" {
                    while names.contains(&title.as_str()) {
                        title.push_str(" (message)");
                    }
                }
                self.chooser.addItemWithTitle(&NSString::from_str(&title));
            }
        }
        if pane.row.is_some() && (changed || pane.schema != previous.schema) {
            self.chooser.selectItemAtIndex(pane.schema as NSInteger);
        }
        match (&pane.json, &previous.json) {
            (Some(json), Some(shown)) if Arc::ptr_eq(json, shown) => {}
            (Some(json), _) => self.schema.show(json),
            (None, _) => self.schema.show_plain(if pane.row.is_some() { "Formatting…" } else { "" }, None),
        }
    }

    /// Detaches selectors before the target is released.
    pub(super) fn clear_actions(&self) {
        self.list.clear_actions();
        // SAFETY: The pop-up accepts a nil target, and the target is still alive.
        unsafe { NSControl::setTarget(&self.chooser, None) };
    }
}

/// The text for a cell at this row and column.
pub(super) fn cell_text(view: &TypesView, row: NSInteger, column: &str) -> Option<(String, String)> {
    let row = view.rows.get(usize::try_from(row).ok()?)?;
    let (text, full) = match column {
        "name" => (row.name.clone(), row.name.clone()),
        "hash" => {
            let short: String = row.hash.chars().take(19).collect();
            let text = if short.len() < row.hash.len() { format!("{short}…") } else { short };
            (text, row.hash.clone())
        }
        _ => {
            let count = row.messages.len().to_string();
            (count.clone(), count)
        }
    };
    Some((text, full))
}
