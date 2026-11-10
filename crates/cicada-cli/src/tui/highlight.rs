//! Coloured JSON and YAML for the describe view and the overview panes.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

const KEY: Color = Color::LightBlue;
const STRING: Color = Color::Green;
const NUMBER: Color = Color::Magenta;
const LITERAL: Color = Color::Yellow;
const PUNCTUATION: Color = Color::DarkGray;

/// `value` as pretty JSON, one coloured line per line.
pub fn json(value: &Value) -> Vec<Line<'static>> {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    text.lines().map(json_line).collect()
}

/// `value` as YAML, one coloured line per line.
pub fn yaml(value: &Value) -> Vec<Line<'static>> {
    let text = serde_norway::to_string(value).unwrap_or_default();
    text.lines().map(yaml_line).collect()
}

fn span(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color))
}

fn json_line(line: &str) -> Line<'static> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let text: String = chars[start..i].iter().collect();
            let is_key = chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&':');
            spans.push(span(text, if is_key { KEY } else { STRING }));
        } else if c == '-' || c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || "+-.".contains(chars[i])) {
                i += 1;
            }
            spans.push(span(chars[start..i].iter().collect::<String>(), NUMBER));
        } else if c.is_alphabetic() {
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            spans.push(span(chars[start..i].iter().collect::<String>(), LITERAL));
        } else if c.is_whitespace() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            spans.push(Span::raw(chars[start..i].iter().collect::<String>()));
        } else {
            i += 1;
            spans.push(span(c.to_string(), PUNCTUATION));
        }
    }
    Line::from(spans)
}

fn yaml_line(line: &str) -> Line<'static> {
    let body = line.trim_start();
    let mut spans = vec![Span::raw(line[..line.len() - body.len()].to_owned())];
    let mut rest = body;
    while let Some(after) = rest.strip_prefix("- ") {
        spans.push(span("- ", PUNCTUATION));
        rest = after;
    }
    if rest == "-" {
        spans.push(span("-", PUNCTUATION));
        return Line::from(spans);
    }
    let split =
        rest.find(": ").map(|i| (i, i + 2)).or_else(|| rest.ends_with(':').then(|| (rest.len() - 1, rest.len())));
    match split {
        Some((colon, value)) if !rest.starts_with(['\'', '"']) => {
            spans.push(span(&rest[..colon], KEY));
            spans.push(span(":", PUNCTUATION));
            let scalar = &rest[value..];
            if !scalar.is_empty() {
                spans.push(Span::raw(" "));
                spans.push(yaml_scalar(scalar));
            }
        }
        _ => spans.push(yaml_scalar(rest)),
    }
    Line::from(spans)
}

fn yaml_scalar(text: &str) -> Span<'static> {
    let color = match text {
        "true" | "false" | "null" | "~" | "[]" | "{}" => LITERAL,
        t if t.parse::<f64>().is_ok() => NUMBER,
        _ => STRING,
    };
    span(text, color)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn colors(line: &Line) -> Vec<(String, Option<Color>)> {
        line.spans
            .iter()
            .filter(|s| !s.content.trim().is_empty())
            .map(|s| (s.content.to_string(), s.style.fg))
            .collect()
    }

    #[test]
    fn json_keys_strings_numbers_and_literals_differ() {
        let lines = json(&json!({"name": "a:b", "n": -1.5, "ok": true}));
        assert_eq!(
            colors(&lines[1]),
            [
                ("\"name\"".into(), Some(KEY)),
                (":".into(), Some(PUNCTUATION)),
                ("\"a:b\"".into(), Some(STRING)),
                (",".into(), Some(PUNCTUATION))
            ]
        );
        assert_eq!(colors(&lines[2])[2], ("-1.5".into(), Some(NUMBER)));
        assert_eq!(colors(&lines[3])[2], ("true".into(), Some(LITERAL)));
    }

    #[test]
    fn yaml_keys_and_scalars_differ() {
        let lines = yaml(&json!({"runs": [{"label": "x", "n": 3}], "paused": false}));
        let text: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        assert_eq!(text, ["runs:", "- label: x", "  n: 3", "paused: false"]);
        assert_eq!(
            colors(&lines[1]),
            [
                ("- ".into(), Some(PUNCTUATION)),
                ("label".into(), Some(KEY)),
                (":".into(), Some(PUNCTUATION)),
                ("x".into(), Some(STRING))
            ]
        );
        assert_eq!(colors(&lines[3])[2], ("false".into(), Some(LITERAL)));
    }
}
