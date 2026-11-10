//! A table's data and view state: rows, live filter, sort and selection.
//!
//! Rows keep their narrow and wide cells (from [`crate::output::Columns`]); the wide cells start with the narrow
//! columns and hold full instants, so they double as sort keys. A refresh replaces the rows but keeps the filter, the
//! sort and the selected item.

use std::cmp::Ordering;

use super::app::Item;

/// One row.
#[derive(Debug, Clone, PartialEq)]
pub struct GridRow {
    /// What the row stands for.
    pub item: Item,
    /// The narrow cells.
    pub cells: Vec<String>,
    /// The wide cells; the first ones are the narrow columns with sortable values.
    pub wide: Vec<String>,
}

impl GridRow {
    /// What the filter searches: the narrow cells and the extra wide ones, never the hidden full instants.
    fn searchable(&self) -> impl Iterator<Item = &String> {
        self.cells.iter().chain(self.wide.iter().skip(self.cells.len()))
    }
}

/// The rows of a table as fetched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rows {
    /// Narrow column names.
    pub headers: Vec<&'static str>,
    /// Wide column names; start with [`Self::headers`].
    pub wide_headers: Vec<&'static str>,
    /// The rows, in the API's order.
    pub rows: Vec<GridRow>,
    /// True when the rows are finished messages (retry applies, cancel does not).
    pub finished: bool,
}

/// A table with its view state.
#[derive(Debug, Clone, Default)]
pub struct Grid {
    /// The data.
    pub data: Rows,
    /// Case-insensitive text every shown row contains.
    pub filter: String,
    /// The sort column and whether it is descending; `None` keeps the API's order.
    pub sort: Option<(usize, bool)>,
    /// The selected position among the shown rows.
    pub selected: usize,
    /// The first shown row on screen, kept by the drawing code.
    pub offset: usize,
    /// True once rows arrived.
    pub loaded: bool,
}

impl Grid {
    /// Replaces the rows, keeping filter, sort and the selected item (or the position when it is gone).
    pub fn set(&mut self, data: Rows) {
        let keep = self.selected_item().cloned();
        let columns = data.wide_headers.len();
        self.data = data;
        self.loaded = true;
        if self.sort.is_some_and(|(c, _)| c >= columns) {
            self.sort = None;
        }
        let shown = self.shown();
        if let Some(position) = keep.and_then(|k| shown.iter().position(|&i| self.data.rows[i].item == k)) {
            self.selected = position;
        }
        self.clamp();
    }

    /// The indices of the rows shown: filtered, then sorted.
    pub fn shown(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        let mut shown: Vec<usize> = (0..self.data.rows.len())
            .filter(|&i| {
                needle.is_empty() || self.data.rows[i].searchable().any(|c| c.to_lowercase().contains(&needle))
            })
            .collect();
        if let Some((column, descending)) = self.sort {
            shown.sort_by(|&a, &b| {
                let key = |i: usize| self.data.rows[i].wide.get(column).map(String::as_str).unwrap_or_default();
                let order = natural(key(a), key(b));
                if descending { order.reverse() } else { order }
            });
        }
        shown
    }

    /// The selected row's item.
    pub fn selected_item(&self) -> Option<&Item> {
        self.shown().get(self.selected).map(|&i| &self.data.rows[i].item)
    }

    /// Moves the selection by `delta` rows, within the shown rows.
    pub fn step(&mut self, delta: isize) {
        self.selected = self.selected.saturating_add_signed(delta);
        self.clamp();
    }

    /// Selects the first (`false`) or last (`true`) row.
    pub fn jump(&mut self, last: bool) {
        self.selected = if last { usize::MAX } else { 0 };
        self.clamp();
    }

    /// Selects shown row `position`.
    pub fn select(&mut self, position: usize) {
        self.selected = position;
        self.clamp();
    }

    /// Sets the filter and selects the first match.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
    }

    /// Sorts by `column`; sorting by the sorted column again flips the order.
    pub fn sort_by(&mut self, column: usize) {
        self.sort = match self.sort {
            Some((c, descending)) if c == column => Some((c, !descending)),
            _ => Some((column, false)),
        };
    }

    /// Moves the sort to the next (`1`) or previous (`-1`) of `columns` columns.
    pub fn sort_step(&mut self, delta: isize, columns: usize) {
        if columns == 0 {
            return;
        }
        let next = match self.sort {
            None if delta < 0 => columns - 1,
            None => 0,
            Some((c, _)) => (c as isize + delta).rem_euclid(columns as isize) as usize,
        };
        self.sort = Some((next, self.sort.is_some_and(|(_, d)| d)));
    }

    /// Flips the sort order, or sorts by the first column.
    pub fn flip(&mut self) {
        self.sort = Some(self.sort.map_or((0, true), |(c, d)| (c, !d)));
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.shown().len().saturating_sub(1));
    }
}

/// Compares numbers as numbers (`9 < 10`) and anything else as text.
fn natural(a: &str, b: &str) -> Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use cicada::store::MessageId;

    use super::*;

    fn row(id: &str, n: &str) -> GridRow {
        let cells = vec![id.to_owned(), n.to_owned()];
        GridRow { item: Item::Message(MessageId::new(id).unwrap()), wide: cells.clone(), cells }
    }

    fn grid(rows: Vec<GridRow>) -> Grid {
        let mut grid = Grid::default();
        grid.set(Rows { headers: vec!["ID", "N"], wide_headers: vec!["ID", "N"], rows, finished: false });
        grid
    }

    fn ids(grid: &Grid) -> Vec<String> {
        grid.shown().iter().map(|&i| grid.data.rows[i].cells[0].clone()).collect()
    }

    #[test]
    fn sorts_numbers_as_numbers_and_flips() {
        let mut g = grid(vec![row("a", "10"), row("b", "9"), row("c", "100")]);
        g.sort_by(1);
        assert_eq!(ids(&g), ["b", "a", "c"]);
        g.sort_by(1);
        assert_eq!(ids(&g), ["c", "a", "b"]);
        g.sort_step(-1, 2);
        assert_eq!(g.sort, Some((0, true)));
    }

    #[test]
    fn filters_case_insensitively() {
        let mut g = grid(vec![row("Alpha", "1"), row("beta", "2")]);
        g.set_filter("ALP".into());
        assert_eq!(ids(&g), ["Alpha"]);
    }

    #[test]
    fn the_filter_ignores_hidden_sort_keys() {
        let mut g = grid(vec![GridRow { wide: vec!["a".into(), "2026-11-10T13:00:00Z".into()], ..row("a", "3m") }]);
        g.set_filter("13:00".into());
        assert!(ids(&g).is_empty());
        g.set_filter("3m".into());
        assert_eq!(ids(&g), ["a"]);
    }

    #[test]
    fn a_refresh_keeps_the_selected_item() {
        let mut g = grid(vec![row("a", "1"), row("b", "2"), row("c", "3")]);
        g.step(1);
        g.set(Rows { rows: vec![row("x", "0"), row("b", "2")], ..g.data.clone() });
        assert_eq!(g.selected, 1);
        g.set(Rows { rows: vec![row("x", "0")], ..g.data.clone() });
        assert_eq!(g.selected, 0, "a vanished item leaves the selection in range");
    }
}
