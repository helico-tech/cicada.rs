//! How results are shown: the columns of each API view (shared by the CLI table and the TUI), the table printer, JSON
//! and YAML.

use std::io::Write;

use chrono::Utc;
use cicada::ActorAddress;
use cicada::api::{
    ActorView, MailboxMessageView, MessageStatusView, MessageView, NodeDetailView, NodeView, OutcomeView,
    RunningTurnView, StatsView, TypeView, WatchView,
};
use cicada::time::{Timestamp, format_instant};
use clap::ValueEnum;
use serde::Serialize;

/// The `-o` formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Aligned columns.
    Table,
    /// Aligned columns, more of them, and full instants.
    Wide,
    /// The API's JSON view, pretty-printed.
    Json,
    /// The API's view as YAML.
    Yaml,
}

/// The table columns of a view.
pub trait Columns {
    /// The column names.
    fn headers(wide: bool) -> Vec<&'static str>;
    /// This item's cells, one per header. Instants are an age relative to `now`, or full ISO-8601 when `wide`.
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String>;
}

/// Writes `items` in `format`: a table, or a JSON / YAML array.
pub fn list<T: Columns + Serialize>(out: &mut (dyn Write + Send), format: Format, items: &[T]) -> anyhow::Result<()> {
    match format {
        Format::Table | Format::Wide => {
            let wide = format == Format::Wide;
            let now = Utc::now();
            let rows: Vec<_> = items.iter().map(|i| i.cells(wide, now)).collect();
            out.write_all(table(&T::headers(wide), &rows).as_bytes())?;
        }
        Format::Json | Format::Yaml => structured(out, format, items)?,
    }
    Ok(())
}

/// Writes one item in `format`; a table has one row.
pub fn one<T: Columns + Serialize>(out: &mut (dyn Write + Send), format: Format, item: &T) -> anyhow::Result<()> {
    match format {
        Format::Table | Format::Wide => list(out, format, std::slice::from_ref(item)),
        Format::Json | Format::Yaml => structured(out, format, item),
    }
}

/// Writes `value` as pretty JSON, or as YAML for any other format.
pub fn structured<T: Serialize + ?Sized>(
    out: &mut (dyn Write + Send),
    format: Format,
    value: &T,
) -> anyhow::Result<()> {
    if format == Format::Json {
        writeln!(out, "{}", serde_json::to_string_pretty(value)?)?;
    } else {
        write!(out, "{}", serde_norway::to_string(value)?)?;
    }
    Ok(())
}

/// Aligns `rows` under `headers`, two spaces between columns, no trailing spaces.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells.iter().zip(&widths).map(|(c, w)| format!("{c:<w$}")).collect();
        padded.join("  ").trim_end().to_owned() + "\n"
    };
    let mut text = line(headers.to_vec());
    for row in rows {
        text += &line(row.iter().map(String::as_str).collect());
    }
    text
}

/// A short age: `5s`, `3m`, `2h`, `4d` for a past instant, `in 5s` for a future one.
pub fn age(instant: Timestamp, now: Timestamp) -> String {
    let seconds = (now - instant).num_seconds();
    let (n, future) = (seconds.unsigned_abs(), seconds < 0);
    let short = match n {
        0..60 => format!("{n}s"),
        60..3600 => format!("{}m", n / 60),
        3600..172_800 => format!("{}h", n / 3600),
        _ => format!("{}d", n / 86_400),
    };
    if future { format!("in {short}") } else { short }
}

fn when(instant: Timestamp, wide: bool, now: Timestamp) -> String {
    if wide { format_instant(instant) } else { age(instant, now) }
}

fn maybe_when(instant: Option<Timestamp>, wide: bool, now: Timestamp) -> String {
    instant.map(|i| when(i, wide, now)).unwrap_or_else(|| "-".into())
}

/// The first line of `text`, cut to 60 characters unless `wide`; `-` for none.
fn brief(text: Option<&str>, wide: bool) -> String {
    let Some(text) = text else { return "-".into() };
    let first = text.lines().next().unwrap_or_default();
    match first.char_indices().nth(60) {
        Some((cut, _)) if !wide => format!("{}…", &first[..cut]),
        _ => first.to_owned(),
    }
}

/// `text` for one cell of a table: each control character escaped (`\n`, `\r`, `\t`, else `\u{hex}`), so that a value
/// cannot break the row; `-` for none.
fn escaped(text: Option<&str>) -> String {
    let Some(text) = text else { return "-".into() };
    let mut cell = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => cell.push_str("\\n"),
            '\r' => cell.push_str("\\r"),
            '\t' => cell.push_str("\\t"),
            c if c.is_control() => cell.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => cell.push(c),
        }
    }
    cell
}

fn dash(text: Option<impl ToString>) -> String {
    text.map(|t| t.to_string()).unwrap_or_else(|| "-".into())
}

fn wide_cols(mut base: Vec<&'static str>, extra: &[&'static str], wide: bool) -> Vec<&'static str> {
    if wide {
        base.extend(extra);
    }
    base
}

impl Columns for ActorAddress {
    fn headers(_: bool) -> Vec<&'static str> {
        vec!["ADDRESS", "TYPE", "KEY"]
    }
    fn cells(&self, _: bool, _: Timestamp) -> Vec<String> {
        vec![self.to_string(), self.type_name().to_string(), self.key().to_owned()]
    }
}

impl Columns for ActorView {
    fn headers(wide: bool) -> Vec<&'static str> {
        wide_cols(vec!["ADDRESS", "VERSION", "PAUSED", "AGE"], &["PARENT", "STATE"], wide)
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        let mut cells = vec![
            self.address.to_string(),
            self.version.to_string(),
            self.paused.to_string(),
            when(self.created_at, wide, now),
        ];
        if wide {
            cells.extend([dash(self.parent.as_ref()), self.state.to_string()]);
        }
        cells
    }
}

impl Columns for WatchView {
    fn headers(_: bool) -> Vec<&'static str> {
        vec!["TARGET", "WATCHER"]
    }
    fn cells(&self, _: bool, _: Timestamp) -> Vec<String> {
        vec![self.target.to_string(), self.watcher.to_string()]
    }
}

impl Columns for MessageView {
    fn headers(wide: bool) -> Vec<&'static str> {
        let extra = ["LOST", "FROM", "CAUSED-BY", "NOTIFICATION", "LAST-ERROR"];
        wide_cols(vec!["ID", "TO", "TYPE", "DUE", "ATTEMPT", "FAILURES"], &extra, wide)
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        let mut cells = vec![
            self.id.to_string(),
            self.to.to_string(),
            self.message_type.clone(),
            when(self.deliver_at, wide, now),
            self.attempt.to_string(),
            self.failures.to_string(),
        ];
        if wide {
            cells.extend([
                self.lost_claims.to_string(),
                dash(self.from.as_ref()),
                dash(self.caused_by.as_ref()),
                self.notification.to_string(),
                brief(self.last_error.as_deref(), true),
            ]);
        }
        cells
    }
}

impl Columns for MailboxMessageView {
    fn headers(wide: bool) -> Vec<&'static str> {
        let mut headers = vec!["STATE"];
        headers.extend(MessageView::headers(wide));
        headers
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        let mut cells = vec![wire(&self.state)];
        cells.extend(self.message.cells(wide, now));
        cells
    }
}

impl Columns for MessageStatusView {
    fn headers(wide: bool) -> Vec<&'static str> {
        let extra = ["NODE", "LEASE-UNTIL", "NOTIFICATION"];
        wide_cols(vec!["ID", "TO", "TYPE", "STATUS", "FAILURES", "FINISHED", "ERROR"], &extra, wide)
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        let mut cells = vec![
            self.id.to_string(),
            self.message.to.to_string(),
            self.message.message_type.clone(),
            wire(&self.status),
            self.failures.to_string(),
            maybe_when(self.finished_at, wide, now),
            brief(self.error.as_deref().or(self.last_error.as_deref()), wide),
        ];
        if wide {
            cells.extend([
                dash(self.node.as_ref()),
                maybe_when(self.lease_until, true, now),
                self.message.notification.to_string(),
            ]);
        }
        cells
    }
}

impl Columns for OutcomeView {
    fn headers(_: bool) -> Vec<&'static str> {
        vec!["ID", "ACTOR", "TYPE", "STATUS", "FINISHED", "ERROR"]
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.actor.to_string(),
            self.message.message_type.clone(),
            wire(&self.status),
            when(self.finished_at, wide, now),
            brief(self.error.as_deref(), wide),
        ]
    }
}

impl Columns for RunningTurnView {
    fn headers(_: bool) -> Vec<&'static str> {
        vec!["ACTOR", "MESSAGE", "NODE", "LEASE", "CANCEL-REQUESTED"]
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        vec![
            self.actor.to_string(),
            self.message_id.to_string(),
            self.node.to_string(),
            when(self.lease_until, wide, now),
            self.cancel_requested.to_string(),
        ]
    }
}

impl Columns for NodeView {
    fn headers(wide: bool) -> Vec<&'static str> {
        wide_cols(vec!["ID", "HOSTNAME", "STATUS", "TURNS", "LAST-SEEN", "AGE"], &["HOSTED", "VERSION", "LABEL"], wide)
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        let mut cells = vec![
            self.id.to_string(),
            escaped(self.hostname.as_deref()),
            wire(&self.status),
            self.running_turns.to_string(),
            when(self.last_seen, wide, now),
            when(self.started_at, wide, now),
        ];
        if wide {
            cells.extend([
                self.hosted.join(","),
                escaped(self.cicada_version.as_deref()),
                escaped(self.label.as_deref()),
            ]);
        }
        cells
    }
}

/// The node's row; JSON and YAML show the whole view, with its turns.
impl Columns for NodeDetailView {
    fn headers(wide: bool) -> Vec<&'static str> {
        NodeView::headers(wide)
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        self.node.cells(wide, now)
    }
}

impl Columns for TypeView {
    fn headers(wide: bool) -> Vec<&'static str> {
        wide_cols(vec!["NAME", "MESSAGES"], &["HASH"], wide)
    }
    fn cells(&self, wide: bool, _: Timestamp) -> Vec<String> {
        let messages = if wide {
            self.messages.keys().cloned().collect::<Vec<_>>().join(",")
        } else {
            self.messages.len().to_string()
        };
        let mut cells = vec![self.name.clone(), messages];
        if wide {
            cells.push(self.hash.clone());
        }
        cells
    }
}

impl Columns for StatsView {
    fn headers(_: bool) -> Vec<&'static str> {
        vec!["ACTORS", "PENDING", "OLDEST-DUE"]
    }
    fn cells(&self, wide: bool, now: Timestamp) -> Vec<String> {
        vec![self.actors.to_string(), self.pending.to_string(), maybe_when(self.oldest_due, wide, now)]
    }
}

/// The wire name of a unit enum value, e.g. `pending` or `alive`.
pub(crate) fn wire(value: &impl Serialize) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    fn now() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
    }

    #[test]
    fn ages_are_short_and_say_when_they_are_ahead() {
        let cases =
            [(0, "0s"), (59, "59s"), (60, "1m"), (3599, "59m"), (3600, "1h"), (47 * 3600, "47h"), (2 * 86_400, "2d")];
        for (seconds, expected) in cases {
            assert_eq!(age(now() - Duration::seconds(seconds), now()), expected, "{seconds}s ago");
        }
        assert_eq!(age(now() + Duration::seconds(90), now()), "in 1m");
    }

    #[test]
    fn a_table_aligns_columns_without_trailing_spaces() {
        let rows = vec![vec!["account/acme:7".to_owned(), "account".to_owned(), "acme:7".to_owned()]];
        let text = table(&ActorAddress::headers(false), &rows);
        assert_eq!(text, "ADDRESS         TYPE     KEY\naccount/acme:7  account  acme:7\n");
    }

    #[test]
    fn views_fill_one_cell_per_header() {
        let address = ActorAddress::parse("account/acme:7").unwrap();
        let actor = ActorView {
            address: address.clone(),
            type_name: "account".into(),
            state: serde_json::json!({"balance": 5}),
            version: 2,
            created_at: now() - Duration::minutes(3),
            paused: false,
            parent: Some(ActorAddress::parse("tenant/acme").unwrap()),
        };
        assert_eq!(actor.cells(false, now()), ["account/acme:7", "2", "false", "3m"]);
        for wide in [false, true] {
            assert_eq!(ActorView::headers(wide).len(), actor.cells(wide, now()).len());
        }
        assert_eq!(actor.cells(true, now())[3], "2026-11-10T11:57:00Z");
        assert_eq!(ActorView::headers(true)[4], "PARENT");
        assert_eq!(actor.cells(true, now())[4], "tenant/acme");
        let independent = ActorView { parent: None, ..actor.clone() };
        assert_eq!(independent.cells(true, now())[4], "-");
        assert_eq!(address.cells(false, now()), ["account/acme:7", "account", "acme:7"]);
    }

    /// A node of a newer server with details, and the same node as an older server serves it.
    pub(crate) fn nodes(now: Timestamp) -> (NodeView, NodeView) {
        let node = NodeView {
            id: cicada::store::NodeId::new("n1").unwrap(),
            hosted: vec!["account".into(), "ledger".into()],
            started_at: now - Duration::hours(2),
            last_seen: now - Duration::seconds(4),
            status: cicada::runtime::NodeHealth::Alive,
            running_turns: 3,
            hostname: Some("web-1.östra".into()),
            cicada_version: Some("0.19.0".into()),
            label: Some(" billing\n2.4\t\r\u{7}\u{9b} 🦀".into()),
        };
        let old = NodeView { hostname: None, cicada_version: None, label: None, ..node.clone() };
        (node, old)
    }

    /// Scenarios: Node list and detail output; Control characters in display metadata.
    #[test]
    fn a_node_shows_its_host_name_and_wide_its_version_and_label_escaped() {
        let (node, old) = nodes(now());
        assert_eq!(NodeView::headers(false), ["ID", "HOSTNAME", "STATUS", "TURNS", "LAST-SEEN", "AGE"]);
        assert_eq!(node.cells(false, now()), ["n1", "web-1.östra", "alive", "3", "4s", "2h"]);
        assert_eq!(old.cells(false, now()), ["n1", "-", "alive", "3", "4s", "2h"]);
        let wide = NodeView::headers(true);
        assert_eq!(wide[..6], NodeView::headers(false)[..], "the wide columns start with the narrow ones");
        assert_eq!(wide[6..], ["HOSTED", "VERSION", "LABEL"]);
        let cells = node.cells(true, now());
        assert_eq!(cells[6..], ["account,ledger", "0.19.0", " billing\\n2.4\\t\\r\\u{7}\\u{9b} 🦀"]);
        assert_eq!(old.cells(true, now())[6..], ["account,ledger", "-", "-"]);
        let detail = NodeDetailView { node: node.clone(), turns: vec![] };
        assert_eq!(NodeDetailView::headers(true), wide);
        assert_eq!(detail.cells(true, now()), cells);

        // A table row stays one line; JSON keeps the text and leaves out what is absent.
        let mut out = Vec::new();
        list(&mut out, Format::Wide, &[node.clone(), old.clone()]).unwrap();
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 3);
        let json = serde_json::to_value(&node).unwrap();
        assert_eq!(json["label"], " billing\n2.4\t\r\u{7}\u{9b} 🦀");
        assert_eq!((json["hostname"].as_str(), json["cicadaVersion"].as_str()), (Some("web-1.östra"), Some("0.19.0")));
        let json = serde_json::to_value(&old).unwrap();
        assert!(["hostname", "cicadaVersion", "label"].iter().all(|f| json.get(f).is_none()), "{json}");
    }

    #[test]
    fn a_cell_escapes_control_characters_and_keeps_printable_unicode() {
        assert_eq!(escaped(None), "-");
        assert_eq!(escaped(Some("a\nb\rc\td\u{0}e\u{1b}f\u{7f}g")), "a\\nb\\rc\\td\\u{0}e\\u{1b}f\\u{7f}g");
        assert_eq!(escaped(Some("Zürich 東京 🦀 \\n")), "Zürich 東京 🦀 \\n", "a backslash is not a control character");
    }

    #[test]
    fn long_errors_are_cut_unless_wide() {
        let long = "x".repeat(80) + "\nsecond line";
        assert_eq!(brief(Some(&long), false).chars().count(), 61);
        assert_eq!(brief(Some(&long), true), "x".repeat(80));
        assert_eq!(brief(None, false), "-");
    }
}
