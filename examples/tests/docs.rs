//! Checks of the docs that need no compiler: the table of guarantees (`docs/guarantees.md`) names, for each guarantee of
//! the architecture, tests that exist, and the relative links between the Markdown files of the repository resolve to a
//! file and, with an anchor, to a heading of it.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// The root of the repository.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// The table of guarantees.

/// The guarantees every table must have a row for, whatever the architecture says: a table the check cannot read then
/// fails for each of them.
const REQUIRED: [&str; 16] =
    ["G1", "G2", "G3", "G4", "G5", "G6", "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9", "S10"];

/// The id of a guarantee at the start of a table cell, as `G1 Serial` or `S10 Retention` have it.
fn guarantee_id(cell: &str) -> Option<&str> {
    let word = cell.split_whitespace().next()?;
    let digits = word.strip_prefix('G').or_else(|| word.strip_prefix('S'))?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(word)
}

/// The cells of a table row, without the outer pipes; nothing for a line that is no row.
fn cells(line: &str) -> Option<Vec<&str>> {
    let line = line.trim();
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(str::trim).collect())
}

/// The guarantees the architecture names: the rows of its tables that start with `G<n>` or `S<n>`.
fn guarantees_of(architecture: &str) -> Vec<String> {
    architecture.lines().filter_map(cells).filter_map(|cells| guarantee_id(cells[0]).map(str::to_owned)).collect()
}

/// The rows of the table of guarantees: the id, and each test of the third cell, a code span `path::function`.
fn rows(table: &str) -> Vec<(String, Vec<String>)> {
    table
        .lines()
        .filter_map(cells)
        .filter_map(|cells| {
            let id = guarantee_id(cells[0])?;
            let tests = cells.get(2).map(|c| code_spans(c)).unwrap_or_default();
            Some((id.to_owned(), tests.into_iter().filter(|t| t.contains("::")).collect()))
        })
        .collect()
}

/// The texts of the code spans of `text` (between single backticks).
fn code_spans(text: &str) -> Vec<String> {
    text.split('`').skip(1).step_by(2).map(str::to_owned).collect()
}

/// Whether the Rust source `source` defines a function `name`.
fn defines(source: &str, name: &str) -> bool {
    let needle = format!("fn {name}");
    source.match_indices(&needle).any(|(i, _)| {
        let before = source[..i].chars().next_back();
        let after = source[i + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && matches!(after, Some('(' | '<'))
    })
}

/// What is wrong with `table`, read against `architecture`, with the files of the tests under `root`: a guarantee
/// without a row, a row that names no test, and a named test whose file is missing or holds no function of its name.
fn check_guarantees(table: &str, architecture: &str, root: &Path) -> Vec<String> {
    let rows = rows(table);
    let mut problems = Vec::new();
    let mut wanted: Vec<String> = REQUIRED.iter().map(|id| (*id).to_owned()).collect();
    for id in guarantees_of(architecture) {
        if !wanted.contains(&id) {
            wanted.push(id);
        }
    }
    for id in &wanted {
        match rows.iter().filter(|(row, _)| row == id).count() {
            0 => problems.push(format!("{id} has no row in docs/guarantees.md")),
            1 => {}
            n => problems.push(format!("{id} has {n} rows in docs/guarantees.md")),
        }
    }
    let mut sources = HashMap::new();
    for (id, tests) in &rows {
        if tests.is_empty() {
            problems.push(format!("{id}: the row names no test (`path::function`)"));
        }
        for test in tests {
            let (path, name) = test.rsplit_once("::").expect("a test holds ::");
            let source = sources.entry(path.to_owned()).or_insert_with(|| fs::read_to_string(root.join(path)).ok());
            match source {
                None => problems.push(format!("{id}: `{test}`: there is no file {path}")),
                Some(source) if !defines(source, name) => {
                    problems.push(format!("{id}: `{test}`: {path} holds no fn {name}"))
                }
                Some(_) => {}
            }
        }
    }
    problems
}

#[test]
fn every_guarantee_names_tests_that_exist() {
    let root = root();
    let table = read(&root.join("docs/guarantees.md"));
    let architecture = read(&root.join("docs/architecture.md"));
    let problems = check_guarantees(&table, &architecture, &root);
    assert!(problems.is_empty(), "The table of guarantees is wrong:\n  {}", problems.join("\n  "));
}

/// A root with one Rust file, `tests.rs`, holding `functions`, and a table whose every row names `tests.rs::t` but the
/// row of S5, which names `tests.rs::s5_case`.
fn fixture(functions: &[&str]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let source: String = functions.iter().map(|f| format!("async fn {f}<B>(backend: &B) {{}}\n")).collect();
    fs::write(dir.path().join("tests.rs"), source).unwrap();
    let mut table = String::from("| # | Guarantee | Tests |\n| --- | --- | --- |\n");
    for id in REQUIRED {
        let test = if id == "S5" { "tests.rs::s5_case" } else { "tests.rs::t" };
        table.push_str(&format!("| {id} Name | What it holds, with `code`. | `{test}`<br>`tests.rs::t` |\n"));
    }
    (dir, table)
}

const ARCHITECTURE: &str = "| # | Guarantee |\n| --- | --- |\n| G1 Serial | … |\n| S10 Retention | … |\n";

/// Scenario: A named test is gone.
#[test]
fn a_renamed_test_fails_the_table_and_is_named_with_its_guarantee() {
    let (dir, table) = fixture(&["t", "s5_case"]);
    assert_eq!(check_guarantees(&table, ARCHITECTURE, dir.path()), Vec::<String>::new());

    let (dir, table) = fixture(&["t", "s5_case_renamed"]);
    assert_eq!(
        check_guarantees(&table, ARCHITECTURE, dir.path()),
        ["S5: `tests.rs::s5_case`: tests.rs holds no fn s5_case"]
    );
    let missing = table.replace("tests.rs::s5_case", "gone.rs::s5_case");
    assert_eq!(
        check_guarantees(&missing, ARCHITECTURE, dir.path()),
        ["S5: `gone.rs::s5_case`: there is no file gone.rs"]
    );
}

/// Scenario: A guarantee without a row.
#[test]
fn a_guarantee_of_the_architecture_without_a_row_is_named() {
    let (dir, table) = fixture(&["t", "s5_case"]);
    let architecture = format!("{ARCHITECTURE}| S11 Something new | … |\n");
    assert_eq!(check_guarantees(&table, &architecture, dir.path()), ["S11 has no row in docs/guarantees.md"]);
    let without_tests = table.replace("| `tests.rs::s5_case`<br>`tests.rs::t` |", "| none yet |");
    assert_eq!(
        check_guarantees(&without_tests, ARCHITECTURE, dir.path()),
        ["S5: the row names no test (`path::function`)"]
    );
}

/// A guarantee that the architecture gains passes once the table has a row for it that names a test.
#[test]
fn a_guarantee_added_to_the_architecture_and_the_table_passes() {
    let (dir, table) = fixture(&["t", "s5_case"]);
    let architecture = format!("{ARCHITECTURE}| S11 Something new | … |\n");
    let with_row = format!("{table}| S11 Something new | What it holds. | `tests.rs::t` |\n");
    assert_eq!(check_guarantees(&with_row, &architecture, dir.path()), Vec::<String>::new());
    let twice = format!("{with_row}| S11 Something new | Again. | `tests.rs::t` |\n");
    assert_eq!(check_guarantees(&twice, &architecture, dir.path()), ["S11 has 2 rows in docs/guarantees.md"]);
}

/// Scenario: A table that cannot be read.
#[test]
fn a_table_that_cannot_be_read_fails_for_every_guarantee() {
    let (dir, table) = fixture(&["t", "s5_case"]);
    let as_a_list = table.replace("| ", "- ").replace(" |", "");
    let problems = check_guarantees(&as_a_list, ARCHITECTURE, dir.path());
    let expected: Vec<String> = REQUIRED.iter().map(|id| format!("{id} has no row in docs/guarantees.md")).collect();
    assert_eq!(problems, expected);
}

#[test]
fn a_function_is_found_by_its_whole_name() {
    let source =
        "fn alpha() {}\npub async fn beta<B: X>(b: &B) {}\nfn gamma_long() {}\n    fn delta(scenario in s()) {";
    assert!(defines(source, "alpha") && defines(source, "beta") && defines(source, "delta"));
    assert!(!defines(source, "gamma") && !defines(source, "lpha") && !defines(source, "epsilon"));
}

// The links.

/// The Markdown files of the repository under `root`: not those under `target`, `.git` or `node_modules`, nor the
/// archive of changes (`openspec/changes/archive`) and the worktrees of agents (`.claude/worktrees`), which are no part
/// of it.
fn markdown_files(root: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap_or(&path);
            if path.is_dir() {
                let name = entry.file_name();
                let skipped = ["target", ".git", "node_modules"].iter().any(|n| name == *n)
                    || relative == Path::new("openspec/changes/archive")
                    || relative == Path::new(".claude/worktrees");
                if !skipped {
                    walk(root, &path, out);
                }
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// `text` with every character of the fenced code blocks and the HTML comments replaced by a space, lines kept, so that
/// neither is read as a heading or a link.
fn without_code_blocks(text: &str) -> String {
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
fn without_code_spans(text: &str) -> String {
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
fn anchors(text: &str) -> BTreeSet<String> {
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

/// The targets of the links of a Markdown text, with their line: inline links and images (`[text](target)`, the text
/// on more than one line too) and reference definitions (`[label]: target`), outside code.
fn links(text: &str) -> Vec<(usize, String)> {
    let text = without_code_spans(&without_code_blocks(text));
    let line_of = |i: usize| text[..i].matches('\n').count() + 1;
    let mut out = Vec::new();
    for (i, _) in text.match_indices("](") {
        let rest = &text[i + 2..];
        let target = match rest.strip_prefix('<') {
            Some(angled) => angled.split('>').next().unwrap_or_default().to_owned(),
            None => {
                let mut depth = 0usize;
                rest.chars()
                    .take_while(|c| match c {
                        '(' => {
                            depth += 1;
                            true
                        }
                        ')' if depth == 0 => false,
                        ')' => {
                            depth -= 1;
                            true
                        }
                        c => !c.is_whitespace(),
                    })
                    .collect()
            }
        };
        out.push((line_of(i), target));
    }
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(definition) = trimmed.strip_prefix('[')
            && let Some((label, target)) = definition.split_once("]:")
            && !label.is_empty()
            && let Some(target) = target.split_whitespace().next()
        {
            out.push((line_of(offset), target.trim_matches(['<', '>']).to_owned()));
        }
        offset += line.len();
    }
    out
}

/// `text` with its `%XX` escapes decoded.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What is wrong with the relative links of `files` under `root`: a link to a file that does not exist, and an anchor
/// that names no heading of the Markdown file it points into. A link to the web is not checked.
fn check_links(root: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut anchors_of: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
    for file in files {
        let text = read(file);
        let shown = file.strip_prefix(root).unwrap_or(file).display().to_string();
        for (line, target) in links(&text) {
            let is_web = target.contains("://") || target.starts_with("mailto:") || target.starts_with("tel:");
            if target.is_empty() || is_web {
                continue;
            }
            let (path, anchor) = target.split_once('#').unwrap_or((target.as_str(), ""));
            let path = percent_decoded(path);
            let resolved = if path.is_empty() {
                file.clone()
            } else if let Some(from_root) = path.strip_prefix('/') {
                root.join(from_root)
            } else {
                file.parent().expect("a folder").join(&path)
            };
            if !resolved.exists() {
                problems.push(format!("{shown}:{line}: the link {target} points to a file that does not exist"));
                continue;
            }
            if anchor.is_empty() || resolved.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let key = resolved.canonicalize().unwrap_or(resolved.clone());
            let known = anchors_of.entry(key).or_insert_with(|| anchors(&read(&resolved)));
            if !known.contains(&percent_decoded(anchor)) {
                problems.push(format!("{shown}:{line}: the link {target} names an anchor that no heading has"));
            }
        }
    }
    problems
}

#[test]
fn the_links_between_the_docs_resolve() {
    let root = root();
    let files = markdown_files(&root);
    assert!(files.len() > 20, "the Markdown files are found: {files:?}");
    let problems = check_links(&root, &files);
    assert!(problems.is_empty(), "Links that do not resolve:\n  {}", problems.join("\n  "));
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn check_all(root: &Path) -> Vec<String> {
    check_links(root, &markdown_files(root))
}

/// Scenario: A section moves.
#[test]
fn a_link_to_a_section_that_moved_fails_until_it_points_to_its_new_place() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "README.md", "# Cicada\n\n## The schema check\n\nText.\n");
    write(root, "docs/operations.md", "See [The schema check](../README.md#the-schema-check).\n");
    assert_eq!(check_all(root), Vec::<String>::new());

    write(root, "README.md", "# Cicada\n\nSee [the HTTP guide](docs/guide/http.md).\n");
    write(root, "docs/guide/http.md", "# HTTP\n\n### The schema check\n");
    assert_eq!(
        check_all(root),
        ["docs/operations.md:1: the link ../README.md#the-schema-check names an anchor that no heading has"]
    );

    write(root, "docs/operations.md", "See [The schema\ncheck](guide/http.md#the-schema-check).\n");
    assert_eq!(check_all(root), Vec::<String>::new());
    write(root, "docs/operations.md", "See [the guide](guide/htp.md).\n");
    assert_eq!(check_all(root), ["docs/operations.md:1: the link guide/htp.md points to a file that does not exist"]);
}

/// Scenario: A link to the web.
#[test]
fn a_link_to_the_web_is_not_checked() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "README.md", "[docs](https://example.com/nowhere#gone) and [mail](mailto:nobody@example.com)\n");
    assert_eq!(check_all(dir.path()), Vec::<String>::new());
}

#[test]
fn code_comments_and_archived_changes_are_not_read_for_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let text =
        "`[a](nowhere.md)`\n\n```rust\nlet x = [b](c);\n```\n\n<!-- doctest\n[d](e.md)\n-->\n\n[f]: missing.md\n";
    write(root, "README.md", text);
    write(root, "openspec/changes/archive/old/proposal.md", "[gone](../../../../nowhere.md)\n");
    write(root, "target/doc/x.md", "[gone](nowhere.md)\n");
    assert_eq!(check_all(root), ["README.md:11: the link missing.md points to a file that does not exist"]);
}

#[test]
fn anchors_are_the_slugs_of_github() {
    let text = "# Cicada\n\n## Testing with `TestActors`\n\n## 0.14.1 — unreleased\n\n## Notes\n\n## Notes\n\n\
                ```sh\n# not a heading\n```\n\n### [Link](x.md) and *more*: `a::b`\n";
    let found: Vec<String> = anchors(text).into_iter().collect();
    assert_eq!(
        found,
        ["0141--unreleased", "cicada", "link-and-more-ab", "notes", "notes-1", "testing-with-testactors"]
    );
}
