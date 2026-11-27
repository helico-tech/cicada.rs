//! Keeps a table's leading row in place when AppKit replaces its rows.

use objc2_app_kit::NSTableView;
use objc2_foundation::NSPoint;

/// A row identity and the position of the viewport before a reload.
pub(super) struct Position<K> {
    pub(super) key: Option<K>,
    offset: NSPoint,
    inset: f64,
    clamped_at: Option<f64>,
}

impl<K> Position<K> {
    /// Captures the row crossing the top of the viewport, including the part already scrolled out of view.
    pub(super) fn capture(table: &NSTableView, key: impl FnOnce(usize) -> Option<K>) -> Option<Self> {
        let scroll = table.enclosingScrollView()?;
        let visible = table.visibleRect();
        let rows = table.rowsInRect(visible);
        let (key, inset) = if rows.length > 0 && rows.location < table.numberOfRows() as usize {
            let row = rows.location;
            (key(row), visible.origin.y - table.rectOfRow(row as isize).origin.y)
        } else {
            (None, 0.0)
        };
        Some(Self { key, offset: scroll.contentView().bounds().origin, inset, clamped_at: None })
    }

    /// A user scroll after clamping takes precedence over the position saved for a later page.
    pub(super) fn still_waiting(&self, table: &NSTableView) -> bool {
        self.clamped_at.is_none_or(|y| {
            table.enclosingScrollView().is_some_and(|scroll| (scroll.contentView().bounds().origin.y - y).abs() < 1.0)
        })
    }

    /// Restores the row or the previous offset. Returns true if a later page may let the view reach that offset.
    pub(super) fn restore(&mut self, table: &NSTableView, row: Option<usize>) -> bool {
        let Some(scroll) = table.enclosingScrollView() else { return false };
        // AppKit can defer the new document size until layout after reloadData.
        scroll.layoutSubtreeIfNeeded();
        let clip = scroll.contentView();
        let mut bounds = clip.bounds();
        bounds.origin = self.offset;
        if let Some(row) = row {
            let y = table.rectOfRow(row as isize).origin.y + self.inset;
            bounds.origin.y = clip.bounds().origin.y + y - table.visibleRect().origin.y;
        }
        let bounds = clip.constrainBoundsRect(bounds);
        clip.scrollToPoint(bounds.origin);
        scroll.reflectScrolledClipView(&clip);
        let y = clip.bounds().origin.y;
        self.clamped_at = (row.is_none() && y + 1.0 < self.offset.y).then_some(y);
        self.clamped_at.is_some()
    }
}
