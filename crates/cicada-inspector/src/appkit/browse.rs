//! Table and footer shared by Types and Watches.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSButton, NSControl, NSFont, NSLineBreakMode, NSScrollView,
    NSTableColumn, NSTableColumnResizingOptions, NSTableView, NSTableViewColumnAutoresizingStyle, NSTableViewStyle,
    NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, ns_string};

use super::content::{Style, color, label};

const FOOTER: f64 = 26.0;

/// A browse table with a status line and paging controls.
pub(super) struct BrowseTable {
    /// The table and footer together.
    pub root: Retained<NSView>,
    /// The table inside the scroll view.
    pub table: Retained<NSTableView>,
    status: Retained<NSTextField>,
    retry: Retained<NSButton>,
    more: Option<Retained<NSButton>>,
}

impl BrowseTable {
    /// The table and footer; the selectors on `target` take their button as sender.
    pub(super) fn new(
        mtm: MainThreadMarker,
        target: &AnyObject,
        name: &str,
        columns: &[(&str, &str, f64)],
        retry: Sel,
        more: Option<Sel>,
    ) -> Self {
        let size = NSSize::new(760.0, 600.0);
        let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
        let table = NSTableView::new(mtm);
        for &(identifier, title, width) in columns {
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str(identifier));
            column.setTitle(&NSString::from_str(title));
            column.setWidth(width);
            column.setMinWidth(80.0);
            column.setResizingMask(NSTableColumnResizingOptions::UserResizingMask);
            table.addTableColumn(&column);
        }
        table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle);
        table.setStyle(NSTableViewStyle::Inset);
        table.setUsesAlternatingRowBackgroundColors(true);
        table.setAllowsEmptySelection(true);
        table.setRowHeight(22.0);
        table.setAccessibilityLabel(Some(&NSString::from_str(name)));
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

        // SAFETY: The target implements both actions with the button as sender and clears them on teardown.
        let (retry, more) = unsafe {
            (
                NSButton::buttonWithTitle_target_action(ns_string!("Retry"), Some(target), Some(retry), mtm),
                more.map(|action| {
                    NSButton::buttonWithTitle_target_action(ns_string!("Load more"), Some(target), Some(action), mtm)
                }),
            )
        };
        for (button, width) in std::iter::once((&retry, 72.0)).chain(more.iter().map(|button| (button, 104.0))) {
            button.setFrame(NSRect::new(NSPoint::new(8.0, 2.0), NSSize::new(width, 22.0)));
            button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMaxYMargin);
            button.setHidden(true);
            root.addSubview(button);
        }
        let status = label(mtm, "", NSFont::systemFontOfSize(11.0), Style::Secondary);
        status.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin,
        );
        root.addSubview(&status);
        Self { root, table, status, retry, more }
    }

    /// Shows the current status and at most one footer button.
    pub(super) fn footer(&self, text: &str, style: Style, retry: bool, more: Option<bool>) {
        self.retry.setHidden(!retry);
        if let Some(button) = &self.more {
            button.setHidden(more.is_none());
            button.setEnabled(more.unwrap_or(false));
        }
        let x = if retry {
            88.0
        } else if more.is_some() {
            120.0
        } else {
            12.0
        };
        let width = self.root.frame().size.width;
        self.status.setFrame(NSRect::new(NSPoint::new(x, 5.0), NSSize::new((width - x - 12.0).max(40.0), 16.0)));
        let text = NSString::from_str(text);
        self.status.setStringValue(&text);
        self.status.setToolTip(Some(&text));
        self.status.setTextColor(Some(&color(style)));
    }

    /// The last row in the viewport, if any.
    pub(super) fn last_visible_row(&self) -> Option<usize> {
        let visible = self.table.rowsInRect(self.table.visibleRect());
        let end = visible.location.saturating_add(visible.length).min(self.table.numberOfRows() as usize);
        end.checked_sub(1)
    }

    /// Clears the references to the target while it is still alive.
    pub(super) fn clear_actions(&self) {
        // SAFETY: The target is still alive and these objects accept a nil target, data source and delegate.
        unsafe {
            self.table.setDataSource(None);
            self.table.setDelegate(None);
            NSControl::setTarget(&self.retry, None);
            if let Some(more) = &self.more {
                NSControl::setTarget(more, None);
            }
        }
        self.table.reloadData();
    }
}

/// A table cell with the full value available to VoiceOver and in its tooltip.
pub(super) fn cell(mtm: MainThreadMarker, text: &str, full: &str) -> Retained<NSView> {
    let field = label(mtm, text, NSFont::systemFontOfSize(13.0), Style::Plain);
    let full = NSString::from_str(full);
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    field.setToolTip(Some(&full));
    field.setAccessibilityLabel(Some(&full));
    Retained::into_super(Retained::into_super(field))
}
