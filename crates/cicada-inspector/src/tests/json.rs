//! The pane's JSON formatter: its tokens, its cut at 256 KiB and its cap of 20,000 runs.

use serde_json::{Value, json};

use crate::json::{CUT, Json, MAX_RUNS, Token, format, format_within, utf16_ranges};

/// The text and token of each run.
fn runs(json: &Json) -> Vec<(&str, Token)> {
    json.runs.iter().map(|run| (&json.text[run.start..run.end], run.token)).collect()
}

#[test]
fn nested_json_is_tokenized_into_keys_strings_numbers_literals_and_punctuation() {
    let value = json!({"a": [1, -2.5e-3, true, null], "b": {"c": "x"}, "d": []});
    let json = format(&value);
    assert_eq!(json.text, serde_json::to_string_pretty(&value).unwrap());
    assert!(!json.cut && !json.capped);
    use Token::*;
    assert_eq!(
        runs(&json),
        [
            ("{", Punctuation),
            ("\"a\"", Key),
            (":", Punctuation),
            ("[", Punctuation),
            ("1", Number),
            (",", Punctuation),
            ("-0.0025", Number),
            (",", Punctuation),
            ("true", Literal),
            (",", Punctuation),
            ("null", Literal),
            ("]", Punctuation),
            (",", Punctuation),
            ("\"b\"", Key),
            (":", Punctuation),
            ("{", Punctuation),
            ("\"c\"", Key),
            (":", Punctuation),
            ("\"x\"", String),
            ("}", Punctuation),
            (",", Punctuation),
            ("\"d\"", Key),
            (":", Punctuation),
            ("[", Punctuation),
            ("]", Punctuation),
            ("}", Punctuation),
        ]
    );
}

#[test]
fn escapes_stay_inside_their_string() {
    let value = json!({"k\"ey": "a \"quoted\": \\ value\n", "n": 1e300});
    let json = format(&value);
    let runs = runs(&json);
    assert_eq!(runs[1], ("\"k\\\"ey\"", Token::Key));
    assert_eq!(runs[3], ("\"a \\\"quoted\\\": \\\\ value\\n\"", Token::String));
    assert_eq!(runs[7], ("1e+300", Token::Number));
}

#[test]
fn non_ascii_text_is_cut_on_a_character_boundary_with_a_notice() {
    // `"é€😀…"`: 1 quote byte, then characters of 2, 3 and 4 bytes, repeated.
    let value = Value::String("é€😀".repeat(100));
    let full = serde_json::to_string_pretty(&value).unwrap();
    for cut in 1..40 {
        let json = format_within(&value, cut, MAX_RUNS);
        assert!(json.cut, "cut at {cut}");
        assert!(json.text.len() <= cut && cut - json.text.len() < 4, "cut at {cut}: {} bytes", json.text.len());
        assert!(full.starts_with(&json.text), "cut at {cut}");
        // The string runs to the end of the cut text.
        assert_eq!(runs(&json), [(json.text.as_str(), Token::String)], "cut at {cut}");
    }
    let exact = format_within(&value, full.len(), MAX_RUNS);
    assert!(!exact.cut && exact.text == full);
}

#[test]
fn a_large_state_is_cut_at_256_kib() {
    let value = json!({"log": "ä".repeat(CUT), "after": 1});
    let json = format(&value);
    assert!(json.cut && json.text.len() <= CUT && json.text.len() > CUT - 4, "{} bytes", json.text.len());
    let runs = runs(&json);
    assert_eq!(runs.len(), 4, "the cut string is the last run");
    assert_eq!(runs[3].1, Token::String);
    assert_eq!(json.runs[3].end, json.text.len());
}

#[test]
fn more_than_20000_tokens_leave_the_rest_plain() {
    let value = Value::Array((0..30_000).map(Value::from).collect());
    let json = format(&value);
    assert!(!json.cut && json.capped);
    assert_eq!(json.runs.len(), MAX_RUNS);
    // `[`, `1`, `,`, `2`, `]`: five tokens.
    let small = format_within(&json!([1, 2]), CUT, 4);
    assert!(small.capped && small.runs.len() == 4);
    let exact = format_within(&json!([1, 2]), CUT, 5);
    assert!(!exact.capped && exact.runs.len() == 5, "{:?}", exact.runs);
}

#[test]
fn runs_become_utf16_ranges() {
    let value = json!({"😀": "é", "a": 1});
    let json = format(&value);
    let ranges = utf16_ranges(&json.text, &json.runs);
    let units: Vec<u16> = json.text.encode_utf16().collect();
    for ((location, length, token), run) in ranges.iter().zip(&json.runs) {
        assert_eq!(*token, run.token);
        assert_eq!(String::from_utf16(&units[*location..location + length]).unwrap(), &json.text[run.start..run.end]);
    }
    assert_eq!(ranges.len(), json.runs.len());
}
