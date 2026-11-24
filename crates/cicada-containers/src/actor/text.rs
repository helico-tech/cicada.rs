//! Text made storable where it is read (design decision 8): a container's output and the runtime's words are untrusted,
//! and Cicada refuses a NUL in a state or a message, so a NUL becomes U+FFFD and the text is bounded. Other control
//! characters stay: a terminal's colours are the reader's business.

/// The most bytes a stored reason keeps.
pub(crate) const MAX_REASON_BYTES: usize = 1024;

/// `text` with every NUL replaced by U+FFFD.
fn plain(text: &str) -> String {
    text.replace('\0', "\u{FFFD}")
}

/// Logs as stored: plain, the last `lines` lines, then the last `bytes` bytes, cut on a character boundary.
pub(crate) fn logs(text: &str, lines: u32, bytes: usize) -> String {
    let text = plain(text);
    last_bytes(last_lines(&text, lines), bytes).to_owned()
}

/// A reason as stored: plain, and its first `MAX_REASON_BYTES` bytes, cut on a character boundary, as a reason's start
/// says what happened.
pub(crate) fn reason(text: &str) -> String {
    let text = plain(text);
    first_bytes(&text, MAX_REASON_BYTES).to_owned()
}

/// The last `lines` lines of `text`, with their line ends; a final line end does not start a line.
fn last_lines(text: &str, lines: u32) -> &str {
    let Some(skip) = (lines as usize).checked_sub(1) else {
        return "";
    };
    let body = text.strip_suffix('\n').unwrap_or(text);
    match body.rmatch_indices('\n').nth(skip) {
        Some((at, _)) => &text[at + 1..],
        None => text,
    }
}

/// The last `bytes` bytes of `text` or fewer, starting on a character boundary.
fn last_bytes(text: &str, bytes: usize) -> &str {
    let mut start = text.len().saturating_sub(bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// The first `bytes` bytes of `text` or fewer, ending on a character boundary.
fn first_bytes(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nul_becomes_the_replacement_character() {
        assert_eq!(logs("a\0b\n", 10, 100), "a\u{FFFD}b\n");
        assert_eq!(reason("bad\0name"), "bad\u{FFFD}name");
    }

    #[test]
    fn logs_keep_their_last_lines() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(logs(text, 2, 100), "two\nthree\n");
        assert_eq!(logs(text, 3, 100), text);
        assert_eq!(logs(text, 9, 100), text);
        assert_eq!(logs(text, 0, 100), "");
        assert_eq!(logs("one\ntwo", 1, 100), "two", "no final line end");
        assert_eq!(logs("", 5, 100), "");
    }

    #[test]
    fn logs_keep_their_last_bytes_on_a_character_boundary() {
        assert_eq!(logs("abcdef", 10, 3), "def");
        // `é` is two bytes: a cut inside it moves forward to the next character.
        assert_eq!(logs("aébc", 10, 3), "bc");
        assert_eq!(logs("aébc", 10, 4), "ébc");
        assert_eq!(logs("abc", 10, 0), "");
        // The replacement character counts as its three bytes.
        assert_eq!(logs("ab\0", 10, 3), "\u{FFFD}");
        assert_eq!(logs("ab\0", 10, 2), "");
    }

    #[test]
    fn lines_are_cut_before_bytes() {
        assert_eq!(logs("first line\nsecond\n", 1, 4), "ond\n");
        assert_eq!(logs("a\nb\nc\n", 2, 100), "b\nc\n");
    }

    #[test]
    fn a_reason_keeps_its_first_kibibyte() {
        assert_eq!(reason(&"x".repeat(2000)).len(), MAX_REASON_BYTES);
        let text = format!("{}é", "x".repeat(MAX_REASON_BYTES - 1));
        assert_eq!(reason(&text), "x".repeat(MAX_REASON_BYTES - 1), "a cut inside a character moves back");
        assert_eq!(reason("short"), "short");
    }
}
