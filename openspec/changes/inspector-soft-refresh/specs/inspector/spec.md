## ADDED Requirements

### Requirement: A soft refresh that keeps the view

The paged and cached inspector pages SHALL offer a soft refresh that re-reads the loaded rows up to the old last row
(at most 300 rows, 4 requests at a time, 10 s), replaces rows within that range, keeps rows past it marked as not
refreshed without duplicates, keeps the selection and the pane's content until its new read answers, and leaves pages
and Retries in flight alone. Every reload of a page's table SHALL keep the scroll position.

#### Scenario: A deleted row

- **WHEN** a soft refresh runs on Messages with 150 rows loaded and one of them is gone
- **THEN** that row disappears, the others keep their place, and the selection stays on its row

#### Scenario: Rows past the limit

- **WHEN** 450 rows are loaded and a soft refresh runs
- **THEN** the first 300 are re-read and the last 150 stay, marked as not refreshed, with none shown twice

#### Scenario: A failed soft refresh

- **WHEN** a soft refresh of Nodes fails, or a paged soft refresh misses its deadline
- **THEN** the old rows stay, marked as not refreshed, and the error shows in the footer's tooltip
