//! The pane's JSON: an actor's state as pretty JSON, cut at [`CUT`] bytes, with style runs for its tokens. It runs on
//! a Tokio worker, so the main thread only applies the runs.

use std::io::{self, Write};

use serde_json::Value;

/// The most bytes of pretty JSON shown; the text is cut there, on a UTF-8 character boundary.
pub const CUT: usize = 256 * 1024;
/// The most style runs; the text past the last one is plain.
pub const MAX_RUNS: usize = 20_000;

/// What a run of the text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// An object's key, quotes included.
    Key,
    /// A string value, quotes included.
    String,
    /// A number.
    Number,
    /// `true`, `false` or `null`.
    Literal,
    /// `{`, `}`, `[`, `]`, `,` or `:`.
    Punctuation,
}

/// A styled part of the text, as byte offsets on character boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// The first byte.
    pub start: usize,
    /// The byte after the last.
    pub end: usize,
    /// What the run is.
    pub token: Token,
}

/// A state formatted for the pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Json {
    /// The pretty JSON, at most [`CUT`] bytes.
    pub text: String,
    /// True when the JSON was longer than [`CUT`] bytes and `text` is its start.
    pub cut: bool,
    /// The style runs in text order, at most [`MAX_RUNS`].
    pub runs: Vec<Run>,
    /// True when the text has more tokens than [`MAX_RUNS`]: the rest is plain.
    pub capped: bool,
}

/// Formats `value` within [`CUT`] bytes and [`MAX_RUNS`] runs.
pub fn format(value: &Value) -> Json {
    format_within(value, CUT, MAX_RUNS)
}

/// [`format`] with the bounds given. The serializer stops writing at `cut`, so a large state is never printed whole.
pub(crate) fn format_within(value: &Value, cut: usize, max_runs: usize) -> Json {
    let mut out = Capped { bytes: Vec::new(), limit: cut, cut: false };
    // The only error is the cap's own; a `Value` always serializes.
    let _ = serde_json::to_writer_pretty(&mut out, value);
    let valid = match std::str::from_utf8(&out.bytes) {
        Ok(text) => text.len(),
        Err(error) => error.valid_up_to(),
    };
    out.bytes.truncate(valid);
    let text = String::from_utf8(out.bytes).expect("cut at a character boundary");
    let (runs, capped) = tokenize(&text, max_runs);
    Json { text, cut: out.cut, runs, capped }
}

/// A writer that keeps the first `limit` bytes and then fails, which stops the serializer.
struct Capped {
    bytes: Vec<u8>,
    limit: usize,
    cut: bool,
}

impl Write for Capped {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let room = self.limit - self.bytes.len();
        if data.len() > room {
            self.bytes.extend_from_slice(&data[..room]);
            self.cut = true;
            return Err(io::Error::other("the state is cut"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The runs of pretty JSON, which may be cut anywhere, even inside a string; true when more than `max` were found.
fn tokenize(text: &str, max: usize) -> (Vec<Run>, bool) {
    let bytes = text.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let token = match bytes[i] {
            b' ' | b'\n' | b'\r' | b'\t' => {
                i += 1;
                continue;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        // serde_json escapes only ASCII after a backslash, so this lands on a boundary.
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                i = i.min(bytes.len());
                let after = bytes[i..].iter().position(|b| !b.is_ascii_whitespace()).map(|n| bytes[i + n]);
                if after == Some(b':') { Token::Key } else { Token::String }
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                i += 1;
                Token::Punctuation
            }
            b'-' | b'0'..=b'9' => {
                while i < bytes.len() && matches!(bytes[i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    i += 1;
                }
                Token::Number
            }
            b if b.is_ascii_alphabetic() => {
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                Token::Literal
            }
            _ => {
                // Not produced by serde_json; skip the character.
                i += text[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
        };
        if runs.len() == max {
            return (runs, true);
        }
        runs.push(Run { start, end: i, token });
    }
    (runs, false)
}

/// The runs as UTF-16 `(location, length)` ranges of `text`, as AppKit's text storage counts, in one pass.
pub fn utf16_ranges(text: &str, runs: &[Run]) -> Vec<(usize, usize, Token)> {
    let mut ranges = Vec::with_capacity(runs.len());
    let mut unit = 0;
    let mut chars = text.char_indices().peekable();
    let mut advance = |to: usize| {
        while let Some(&(at, c)) = chars.peek() {
            if at >= to {
                break;
            }
            unit += c.len_utf16();
            chars.next();
        }
        unit
    };
    for run in runs {
        let start = advance(run.start);
        let end = advance(run.end);
        ranges.push((start, end - start, run.token));
    }
    ranges
}
