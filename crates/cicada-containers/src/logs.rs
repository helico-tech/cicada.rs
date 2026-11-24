//! Log pages: [`LogPage`], [`LogLine`], [`LogStream`], the opaque [`LogCursor`], and the private pager that both
//! runtimes feed, so the fake pages exactly as Docker does.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use cicada::time::Timestamp;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{ContainerError, ErrorKind};

/// The most lines one [`logs_since`](crate::ContainerRuntime::logs_since) call returns; `max_lines` is 1 to this.
pub const LOG_PAGE_MAX: u32 = 1000;

/// A page of log lines from [`logs_since`](crate::ContainerRuntime::logs_since), and the cursor to read on from.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPage {
    /// The lines, in log order. Fewer than `max_lines` means the call read everything there is now.
    pub lines: Vec<LogLine>,
    /// The cursor to call again with; the cursor given, or the start, when `lines` is empty.
    pub next: LogCursor,
}

impl LogPage {
    /// A page of `lines` that reads on from `next`.
    pub fn new(lines: Vec<LogLine>, next: LogCursor) -> Self {
        Self { lines, next }
    }
}

/// One log line: its time, its stream and its text as lossy UTF-8 without the newline.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// When the runtime stamped it.
    pub time: Timestamp,
    /// Which stream wrote it.
    pub stream: LogStream,
    /// Its text, without the newline.
    pub text: String,
}

impl LogLine {
    /// The line `text` on `stream`, stamped `time`.
    pub fn new(time: Timestamp, stream: LogStream, text: impl Into<String>) -> Self {
        Self { time, stream, text: text.into() }
    }
}

/// The stream a [`LogLine`] came from.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogStream {
    /// Standard output.
    Out,
    /// Standard error.
    Err,
}

/// Where a [`logs_since`](crate::ContainerRuntime::logs_since) reads on from. Opaque: keep it as the string of
/// `Display` (or serde, which writes the same string) and give it back by `FromStr`; a string that is not a cursor
/// fails with `InvalidSpec`. Its form is kept across the patch releases of a minor version and means the same on
/// every runtime of the crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LogCursor {
    /// The Unix second the read starts from.
    second: i64,
    /// How many lines of the stream from `second` come before the cursor.
    skip: u64,
}

impl LogCursor {
    /// The cursor of the first line.
    pub(crate) const START: Self = Self { second: 0, skip: 0 };

    /// The Unix second a read from this cursor starts from: Docker's `since`.
    #[cfg(any(test, feature = "docker"))]
    pub(crate) fn second(self) -> i64 {
        self.second
    }
}

impl fmt::Display for LogCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.second, self.skip)
    }
}

impl FromStr for LogCursor {
    type Err = ContainerError;

    fn from_str(text: &str) -> Result<Self, ContainerError> {
        let parsed = text
            .split_once(':')
            .and_then(|(second, skip)| Some(Self { second: second.parse().ok()?, skip: skip.parse().ok()? }))
            .filter(|cursor| cursor.to_string() == text); // one spelling per cursor: no `+`, no leading zeros
        parsed.ok_or_else(|| ContainerError::new(ErrorKind::InvalidSpec, format!("{text:?} is not a log cursor")))
    }
}

impl Serialize for LogCursor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LogCursor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(|e: ContainerError| serde::de::Error::custom(e.message))
    }
}

impl JsonSchema for LogCursor {
    fn schema_name() -> Cow<'static, str> {
        "LogCursor".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        String::json_schema(generator)
    }
}

/// `InvalidSpec` for a page size outside 1 to [`LOG_PAGE_MAX`].
pub(crate) fn check_max_lines(name: &str, max_lines: u32) -> Result<(), ContainerError> {
    if (1..=LOG_PAGE_MAX).contains(&max_lines) {
        Ok(())
    } else {
        Err(ContainerError::new(
            ErrorKind::InvalidSpec,
            format!("logs of {name}: max_lines is {max_lines}, not 1 to {LOG_PAGE_MAX}"),
        ))
    }
}

/// Builds a page from the lines of a read from the cursor's second, in log order (design, decisions 1 and 6). It
/// drops every line stamped before that second, whatever the runtime's filter let through, skips the lines the
/// cursor counts and takes up to `max_lines`. The next cursor holds the second of the last line taken and the number
/// of lines up to it, the skipped ones too, stamped at or after that second.
pub(crate) struct Pager {
    cursor: LogCursor,
    max_lines: usize,
    /// Lines kept so far, the skipped ones too.
    seen: u64,
    /// How many of the kept lines fall in each second.
    per_second: BTreeMap<i64, u64>,
    lines: Vec<LogLine>,
}

impl Pager {
    /// A pager from `cursor` (the start for `None`) for pages of up to `max_lines` lines.
    pub(crate) fn new(cursor: Option<LogCursor>, max_lines: u32) -> Self {
        Self {
            cursor: cursor.unwrap_or(LogCursor::START),
            max_lines: max_lines as usize,
            seen: 0,
            per_second: BTreeMap::new(),
            lines: Vec::new(),
        }
    }

    /// Whether the page holds `max_lines` lines: the caller stops reading.
    pub(crate) fn is_full(&self) -> bool {
        self.lines.len() >= self.max_lines
    }

    /// Takes the next line of the read; ignored once the page is full.
    pub(crate) fn push(&mut self, line: LogLine) {
        let second = line.time.timestamp();
        if self.is_full() || second < self.cursor.second {
            return;
        }
        self.seen += 1;
        *self.per_second.entry(second).or_default() += 1;
        if self.seen > self.cursor.skip {
            self.lines.push(line);
        }
    }

    /// The page, and its next cursor.
    pub(crate) fn finish(self) -> LogPage {
        let Some(last) = self.lines.last() else { return LogPage::new(self.lines, self.cursor) };
        let second = last.time.timestamp();
        let skip = self.per_second.range(second..).map(|(_, count)| count).sum();
        LogPage::new(self.lines, LogCursor { second, skip })
    }
}

/// The page of `lines`, a whole read from the cursor's second, as [`Pager`] builds it.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn page(lines: impl IntoIterator<Item = LogLine>, cursor: Option<LogCursor>, max_lines: u32) -> LogPage {
    let mut pager = Pager::new(cursor, max_lines);
    for line in lines {
        if pager.is_full() {
            break;
        }
        pager.push(line);
    }
    pager.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line stamped at Unix second `second` plus `micros`.
    fn at(second: i64, micros: i64, stream: LogStream, text: &str) -> LogLine {
        LogLine::new(cicada::time::from_micros(second * 1_000_000 + micros), stream, text)
    }

    fn out(second: i64, text: &str) -> LogLine {
        at(second, 0, LogStream::Out, text)
    }

    fn texts(page: &LogPage) -> Vec<&str> {
        page.lines.iter().map(|line| line.text.as_str()).collect()
    }

    fn cursor(text: &str) -> LogCursor {
        text.parse().unwrap()
    }

    /// Reads `log` to the end in pages of `max_lines`, as a follower would, each read seeing the lines from the
    /// cursor's second as Docker's filter on older engines gives them.
    fn read_all(log: &[LogLine], max_lines: u32) -> Vec<String> {
        let mut texts = Vec::new();
        let mut next = None;
        loop {
            let from = next.map_or(0, LogCursor::second);
            let read = log.iter().filter(|line| line.time.timestamp() >= from).cloned();
            let page = page(read, next, max_lines);
            texts.extend(page.lines.iter().map(|line| line.text.clone()));
            next = Some(page.next);
            if page.lines.len() < max_lines as usize {
                return texts;
            }
        }
    }

    #[test]
    fn lines_sharing_a_second_come_once_each_in_every_page_size() {
        let log: Vec<LogLine> = (0..50)
            .map(|i| at(100 + i / 20, i, if i % 3 == 0 { LogStream::Err } else { LogStream::Out }, &i.to_string()))
            .collect();
        let all: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        for max_lines in [1, 2, 7, 20, 49, 50, 51, 1000] {
            assert_eq!(read_all(&log, max_lines), all, "max_lines {max_lines}");
        }
    }

    #[test]
    fn lines_out_of_time_order_within_a_second_come_once_each() {
        let log = vec![
            at(100, 500, LogStream::Out, "a"),
            at(100, 100, LogStream::Err, "b"),
            at(101, 0, LogStream::Out, "c"),
            at(101, 900, LogStream::Err, "d"),
            at(101, 200, LogStream::Out, "e"),
            at(102, 0, LogStream::Out, "f"),
        ];
        for max_lines in 1..=7 {
            assert_eq!(read_all(&log, max_lines), ["a", "b", "c", "d", "e", "f"], "max_lines {max_lines}");
        }
    }

    #[test]
    fn a_page_cut_in_a_second_counts_the_lines_of_that_second() {
        let log = vec![out(100, "a"), out(101, "b"), out(101, "c"), out(101, "d")];
        let first = page(log.clone(), None, 3);
        assert_eq!(texts(&first), ["a", "b", "c"]);
        assert_eq!(first.next, cursor("101:2"));
        let read: Vec<LogLine> = log.into_iter().filter(|line| line.time.timestamp() >= 101).collect();
        let second = page(read, Some(first.next), 3);
        assert_eq!(texts(&second), ["d"]);
        assert_eq!(second.next, cursor("101:3"));
    }

    #[test]
    fn the_skipped_lines_count_in_the_next_cursor() {
        // From 101:2, the next page ends in the same second: its count holds the two skipped lines too.
        let read = vec![out(101, "b"), out(101, "c"), out(101, "d"), out(101, "e"), out(102, "f")];
        let page = page(read, Some(cursor("101:2")), 2);
        assert_eq!(texts(&page), ["d", "e"]);
        assert_eq!(page.next, cursor("101:4"));
    }

    #[test]
    fn an_empty_read_keeps_the_cursor() {
        assert_eq!(page(Vec::new(), None, 5), LogPage::new(Vec::new(), cursor("0:0")));
        let given = cursor("101:3");
        let read = vec![out(101, "a"), out(101, "b"), out(101, "c")];
        assert_eq!(page(read, Some(given), 5), LogPage::new(Vec::new(), given));
    }

    #[test]
    fn a_repeated_cursor_gives_the_same_page() {
        let read = vec![out(100, "a"), out(100, "b"), out(101, "c"), out(101, "d")];
        let given = Some(cursor("100:1"));
        assert_eq!(page(read.clone(), given, 2), page(read, given, 2));
    }

    #[test]
    fn a_line_stamped_before_the_cursor_second_is_dropped_and_nothing_comes_again() {
        // A (T), then B stamped T-1 but written after A, then C (T): Docker's `since=T` on moby master gives all three.
        let (a, b, c) = (out(200, "A"), out(199, "B"), out(200, "C"));
        let from_start = page(vec![a.clone(), b.clone(), c.clone()], None, 3);
        assert_eq!(texts(&from_start), ["A", "B", "C"]);
        assert_eq!(from_start.next, cursor("200:2"), "B is before second 200, so it is not counted");
        let again = page(vec![a, b, c, out(200, "D")], Some(from_start.next), 3);
        assert_eq!(texts(&again), ["D"], "B dropped, A and C skipped, C not again");
    }

    #[test]
    fn a_page_stops_at_max_lines() {
        let read: Vec<LogLine> = (0..10).map(|i| out(100, &i.to_string())).collect();
        let mut pager = Pager::new(None, 4);
        for line in read {
            pager.push(line);
        }
        assert!(pager.is_full());
        let page = pager.finish();
        assert_eq!(texts(&page), ["0", "1", "2", "3"]);
        assert_eq!(page.next, cursor("100:4"));
    }

    #[test]
    fn a_cursor_round_trips_through_its_string_and_json() {
        for text in ["0:0", "1764000000:12", "-5:3"] {
            let parsed = cursor(text);
            assert_eq!(parsed.to_string(), text);
            let json = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<LogCursor>(&json).unwrap(), parsed);
        }
        let schema = serde_json::to_value(schemars::schema_for!(LogCursor)).unwrap();
        assert_eq!(schema["type"], "string", "{schema}");
    }

    #[test]
    fn a_string_that_is_not_a_cursor_is_invalid_spec() {
        for text in ["", "5", "5:", ":5", "a:1", "1:b", "1:2:3", "1:-2", "+1:2", "01:2", "1:02", " 1:2"] {
            let error = text.parse::<LogCursor>().unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidSpec, "{text:?}");
        }
        assert!(serde_json::from_str::<LogCursor>("\"x\"").is_err());
        assert!(serde_json::from_str::<LogCursor>("5").is_err());
    }

    #[test]
    fn max_lines_is_one_to_the_bound() {
        assert!(check_max_lines("job-1", 1).is_ok());
        assert!(check_max_lines("job-1", LOG_PAGE_MAX).is_ok());
        for bad in [0, LOG_PAGE_MAX + 1] {
            assert_eq!(check_max_lines("job-1", bad).unwrap_err().kind, ErrorKind::InvalidSpec);
        }
    }
}
