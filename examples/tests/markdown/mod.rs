//! Reading Markdown as GitHub does, for the checks of the docs (`docs.rs`) and of the consumer skill (`skill.rs`):
//! code blocks and spans that hold no headings or links, and the anchors of the headings.

use std::collections::{BTreeSet, HashMap};

/// `text` with every character of the fenced code blocks and the HTML comments replaced by a space, lines kept, so that
/// neither is read as a heading or a link.
pub fn without_code_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<String> = None;
    let mut in_comment = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) && trimmed.trim_end().chars().all(|c| open.starts_with(c)) {
                fence = None;
            }
            out.extend(line.chars().map(|c| if c == '\n' { c } else { ' ' }));
            continue;
        }
        if !in_comment && (trimmed.starts_with("```") || trimmed.starts_with("~~~")) {
            let mark = trimmed.chars().next().unwrap();
            fence = Some(trimmed.chars().take_while(|c| *c == mark).collect());
            out.extend(line.chars().map(|c| if c == '\n' { c } else { ' ' }));
            continue;
        }
        let mut rest = line;
        while !rest.is_empty() {
            if in_comment {
                match rest.find("-->") {
                    Some(end) => {
                        out.extend(rest[..end + 3].chars().map(|c| if c == '\n' { c } else { ' ' }));
                        rest = &rest[end + 3..];
                        in_comment = false;
                    }
                    None => {
                        out.extend(rest.chars().map(|c| if c == '\n' { c } else { ' ' }));
                        rest = "";
                    }
                }
            } else {
                match rest.find("<!--") {
                    Some(start) => {
                        out.push_str(&rest[..start]);
                        rest = &rest[start..];
                        in_comment = true;
                    }
                    None => {
                        out.push_str(rest);
                        rest = "";
                    }
                }
            }
        }
    }
    out
}

/// `text` with the code spans (between runs of backticks of one length) replaced by spaces.
pub fn without_code_spans(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = text.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = bytes[i..].iter().take_while(|b| **b == b'`').count();
        let mut j = i + run;
        let mut close = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let other = bytes[j..].iter().take_while(|b| **b == b'`').count();
                if other == run {
                    close = Some(j);
                    break;
                }
                j += other;
            } else {
                j += 1;
            }
        }
        match close {
            Some(j) => {
                for b in &mut out[i..j + run] {
                    if *b != b'\n' {
                        *b = b' ';
                    }
                }
                i = j + run;
            }
            None => i += run,
        }
    }
    String::from_utf8(out).expect("a span is replaced whole, between ASCII backticks")
}

/// The anchor GitHub gives a heading: its text in lower case, without the characters that are no letter, digit, `_`,
/// `-` or space, each space a `-`.
fn slug(heading: &str) -> String {
    let mut text = String::new();
    let mut rest = heading;
    // A link in a heading counts with its text only.
    while let Some(open) = rest.find('[') {
        match rest[open..].find("](").and_then(|mid| rest[open + mid..].find(')').map(|close| (mid, close))) {
            Some((mid, close)) => {
                text.push_str(&rest[..open]);
                text.push_str(&rest[open + 1..open + mid]);
                rest = &rest[open + mid + close + 1..];
            }
            None => break,
        }
    }
    text.push_str(rest);
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The anchors of a Markdown text: one per heading, `-1`, `-2`, … for a repeated one, and the `id` and `name`
/// attributes of its HTML.
pub fn anchors(text: &str) -> BTreeSet<String> {
    let text = without_code_blocks(text);
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let level = trimmed.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            let base = slug(trimmed[level..].trim().trim_end_matches('#').trim());
            let n = seen.entry(base.clone()).or_insert(0);
            out.insert(if *n == 0 { base.clone() } else { format!("{base}-{n}") });
            *n += 1;
        }
        for attribute in ["id=\"", "name=\""] {
            for (i, _) in line.match_indices(attribute) {
                let value = &line[i + attribute.len()..];
                if let Some(end) = value.find('"') {
                    out.insert(value[..end].to_owned());
                }
            }
        }
    }
    out
}
