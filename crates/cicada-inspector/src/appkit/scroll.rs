//! Keeps a table's leading row in place when AppKit replaces its rows.

use objc2_app_kit::NSTableView;
use objc2_foundation::NSPoint;

/// A row identity and the position of the viewport before a reload.
pub(super) struct Position<K> {
    pub(super) key: Option<K>,
    offset: NSPoint,
    inset: f64,
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
        Some(Self { key, offset: scroll.contentView().bounds().origin, inset })
    }

    /// Restores the row if it survived, or the previous offset within the new scrollable area.
    pub(super) fn restore(&self, table: &NSTableView, row: Option<usize>) {
        let Some(scroll) = table.enclosingScrollView() else { return };
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
    }
}
