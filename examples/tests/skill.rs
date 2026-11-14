//! Checks of the consumer skill `skills/cicada` (ADR 0041): its frontmatter and version, that every route resolves to a
//! file and heading of this source, that the routes cover every guide, reference and worked example, that consumers
//! are not routed to proposals or the change loops, the evaluation cases, and the Python tests of the resolver and the
//! packaging (`examples/tests/skill/test_*.py`). They check structure; whether the skill leads to good decisions is
//! judged by evaluations in review.

#[allow(dead_code)] // the checks of the skill use only the anchors
mod markdown;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use markdown::{anchors, without_code_blocks};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().expect("the repository")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn skill() -> String {
    read(&root().join("skills/cicada/SKILL.md"))
}

/// The heading of the one section that may route to the contributor's instructions.
const SELF_SECTION: &str = "## Working on Cicada itself";

/// The starts of a code span that is a route into the source.
const ROUTE_STARTS: [&str; 9] = [
    "README.md",
    "CHANGELOG.md",
    "AGENTS.md",
    "docs/",
    "examples/",
    "crates/",
    "sample-service/",
    "openspec/",
    ".claude/",
];

/// What a consumer is never routed to: proposals, the change loops and their files.
const CONTRIBUTOR_ONLY: [&str; 8] = [
    "openspec/",
    ".claude/",
    ".reviews",
    "AGENTS.md",
    "scripts/review.py",
    "scripts/status.py",
    "docs/maintaining/",
    "docs/how-to/releasing.md",
];

/// Keys of a frontmatter with their values, in order.
type Pairs = Vec<(String, String)>;

/// The frontmatter's top-level keys with their values, and the keys of `metadata` with theirs.
fn frontmatter(text: &str) -> (Pairs, Pairs) {
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("---"), "SKILL.md starts with a frontmatter");
    let (mut top, mut metadata) = (Vec::new(), Vec::new());
    let mut in_metadata = false;
    for line in lines.by_ref() {
        if line == "---" {
            return (top, metadata);
        }
        let (key, value) = line.trim().split_once(':').unwrap_or((line.trim(), ""));
        let pair = (key.to_owned(), value.trim().trim_matches(['"', '\'']).to_owned());
        if line.starts_with(' ') {
            assert!(in_metadata, "an indented line outside metadata: {line}");
            metadata.push(pair);
        } else {
            in_metadata = key == "metadata";
            top.push(pair);
        }
    }
    panic!("the frontmatter of SKILL.md is not closed");
}

/// `[workspace.package] version` of Cargo.toml.
fn workspace_version(root: &Path) -> String {
    let manifest = read(&root.join("Cargo.toml"));
    let section = manifest.split("[workspace.package]").nth(1).expect("[workspace.package]");
    let line = section.lines().find(|l| l.starts_with("version")).expect("a version");
    line.split('"').nth(1).expect("a quoted version").to_owned()
}

/// The code spans of `text` outside fenced code blocks, with their line.
fn spans(text: &str) -> Vec<(usize, String)> {
    let text = without_code_blocks(text);
    let mut out = Vec::new();
    for (number, line) in text.lines().enumerate() {
        out.extend(line.split('`').skip(1).step_by(2).map(|s| (number + 1, s.to_owned())));
    }
    out
}

/// The routes of `text`: code spans that start like a path of the source.
fn routes(text: &str) -> Vec<(usize, String)> {
    spans(text).into_iter().filter(|(_, s)| ROUTE_STARTS.iter().any(|start| s.starts_with(start))).collect()
}

/// What is wrong with the routes of `text` against the source at `root`: a file that does not exist, or an anchor that
/// no heading of it has.
fn check_routes(text: &str, root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for (line, route) in routes(text) {
        let (path, anchor) = route.split_once('#').unwrap_or((route.as_str(), ""));
        let file = root.join(path);
        if !file.exists() {
            problems.push(format!("SKILL.md:{line}: the route {route} names a file that does not exist"));
        } else if !anchor.is_empty() && !anchors(&read(&file)).contains(anchor) {
            problems.push(format!("SKILL.md:{line}: the route {route} names an anchor that no heading has"));
        }
    }
    problems
}

/// The paths every consumer route set must name: the guides, the how-tos and references a consumer uses (not the
/// release procedure of Cicada itself), and every worked example.
fn required_paths(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = [
        "README.md",
        "CHANGELOG.md",
        "docs/architecture.md",
        "docs/guarantees.md",
        "docs/operations.md",
        "docs/openapi.json",
        "docs/reference/contracts/",
        "examples/tests/",
        "sample-service/",
    ]
    .map(str::to_owned)
    .into();
    for folder in ["docs/guide", "docs/how-to", "docs/reference"] {
        for entry in fs::read_dir(root.join(folder)).unwrap_or_else(|e| panic!("{folder}: {e}")) {
            let path = format!("{folder}/{}", entry.expect("an entry").file_name().to_string_lossy());
            if path.ends_with(".md") && !CONTRIBUTOR_ONLY.iter().any(|bad| path.contains(bad)) {
                paths.push(path);
            }
        }
    }
    for entry in fs::read_dir(root.join("examples/src")).expect("examples/src") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            paths.push(format!("examples/src/{name}/"));
        } else if name.ends_with(".rs") && name != "lib.rs" {
            paths.push(format!("examples/src/{name}"));
        }
    }
    paths.sort();
    paths
}

/// The sections that decisions of a consumer depend on: actor design, external effects, timers, flows, testing, HTTP
/// and its authentication, workers, telemetry, deploys, contract changes and upgrades, and adding a release.
const REQUIRED_ROUTES: [&str; 16] = [
    "README.md#using-a-release-in-another-project",
    "docs/guide/core.md#design-an-actor",
    "docs/guide/core.md#retry",
    "docs/guide/core.md#timers",
    "docs/guide/core.md#testing-with-testactors",
    "docs/guide/core.md#contract-snapshots",
    "docs/guide/core.md#state-samples",
    "docs/guide/core.md#run-actors-in-a-worker",
    "docs/guide/core.md#opentelemetry",
    "docs/guide/actors.md#flows",
    "docs/guide/http.md#run-it-in-an-axum-service",
    "docs/operations.md#deploying",
    "docs/operations.md#changing-a-contract",
    "docs/operations.md#upgrading",
    "docs/operations.md#telemetry",
    "docs/architecture.md#guarantees",
];

/// What the consumer part of `text` (all but the self section) misses of `required` paths and routes.
fn missing_coverage(text: &str, required: &[String]) -> Vec<String> {
    let consumer = text.split(SELF_SECTION).next().unwrap_or(text);
    let named: Vec<String> = routes(consumer).into_iter().map(|(_, r)| r).collect();
    required
        .iter()
        .filter(|want| {
            !named.iter().any(|route| {
                let path = route.split('#').next().unwrap_or_default();
                if want.contains('#') { route == *want } else { path == want.as_str() }
            })
        })
        .map(|want| format!("SKILL.md routes nothing to {want}"))
        .collect()
}

/// Spans of the consumer part of `text` that name a proposal, a loop or the contributor's instructions.
fn contributor_routes(text: &str) -> Vec<String> {
    let consumer = text.split(SELF_SECTION).next().unwrap_or(text);
    spans(consumer)
        .into_iter()
        .filter(|(_, s)| CONTRIBUTOR_ONLY.iter().any(|bad| s.contains(bad)))
        .map(|(line, s)| format!("SKILL.md:{line}: a consumer is routed to {s}"))
        .collect()
}

#[test]
fn the_frontmatter_names_the_skill_and_the_workspace_version() {
    let (top, metadata) = frontmatter(&skill());
    let keys: Vec<&str> = top.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["name", "description", "metadata"], "only these keys, no harness metadata");
    assert_eq!(top[0].1, "cicada");
    let description = &top[1].1;
    assert!(description.len() > 100 && description.len() <= 1024, "a description of at most 1024 characters");
    assert_eq!(metadata.len(), 1, "metadata holds the version only: {metadata:?}");
    assert_eq!(metadata[0].0, "cicada-version");
    assert_eq!(metadata[0].1, workspace_version(&root()), "metadata.cicada-version is the workspace version");
}

#[test]
fn every_route_resolves_to_a_file_and_heading_of_this_source() {
    let text = skill();
    assert!(routes(&text).len() > 50, "the routes are found");
    let problems = check_routes(&text, &root());
    assert!(problems.is_empty(), "Routes that do not resolve:\n  {}", problems.join("\n  "));
}

#[test]
fn the_routes_cover_every_guide_reference_and_example() {
    let root = root();
    let mut required = required_paths(&root);
    required.extend(REQUIRED_ROUTES.map(str::to_owned));
    let problems = missing_coverage(&skill(), &required);
    assert!(problems.is_empty(), "The skill misses:\n  {}", problems.join("\n  "));
}

#[test]
fn consumers_are_not_routed_to_proposals_or_loops() {
    let text = skill();
    let problems = contributor_routes(&text);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let own = text.split(SELF_SECTION).nth(1).expect("the self section");
    assert!(own.contains("`AGENTS.md`"), "the self section points to the repository's instructions");
}

/// Scenario: A stale route or version.
#[test]
fn a_stale_route_or_a_missing_entry_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("docs/guide")).unwrap();
    fs::write(root.join("docs/guide/core.md"), "# Core\n\n## Retry\n").unwrap();
    let text = "Read `docs/guide/core.md#retry`, `docs/guide/core.md#timers` and `docs/guide/gone.md`.\n\n```sh\n\
                `docs/in-a-block.md`\n```\n";
    assert_eq!(
        check_routes(text, root),
        [
            "SKILL.md:1: the route docs/guide/core.md#timers names an anchor that no heading has",
            "SKILL.md:1: the route docs/guide/gone.md names a file that does not exist",
        ]
    );
    let required = ["docs/guide/core.md".to_owned(), "examples/src/new.rs".to_owned()];
    assert_eq!(missing_coverage(text, &required), ["SKILL.md routes nothing to examples/src/new.rs"]);
    let text = format!("Use `openspec/changes/x/design.md`.\n\n{SELF_SECTION}\n\nRead `AGENTS.md`.\n");
    assert_eq!(contributor_routes(&text), ["SKILL.md:1: a consumer is routed to openspec/changes/x/design.md"]);
}

/// The ids of the evaluation cases the design names.
const EVALUATIONS: [&str; 9] = [
    "new-sqlite-integration",
    "duplicate-payment",
    "flow-cancellation",
    "offline-old-source",
    "moved-heading",
    "mixed-fork",
    "http-auth",
    "unshipped-feature",
    "contract-change-deploy",
];

#[test]
fn the_evaluation_cases_are_complete() {
    let text = read(&root().join("examples/tests/skill/evals.json"));
    let evals: serde_json::Value = serde_json::from_str(&text).expect("evals.json is JSON");
    let cases = evals["cases"].as_array().expect("a list of cases");
    let ids: Vec<&str> = cases.iter().filter_map(|c| c["id"].as_str()).collect();
    for id in EVALUATIONS {
        assert!(ids.contains(&id), "evals.json has no case {id}");
    }
    for case in cases {
        for field in ["request", "setup"] {
            assert!(case[field].as_str().is_some_and(|s| !s.is_empty()), "{}: no {field}", case["id"]);
        }
        for field in ["expect", "avoid"] {
            assert!(case[field].as_array().is_some_and(|a| !a.is_empty()), "{}: no {field}", case["id"]);
        }
    }
}

fn python() -> Command {
    let mut command = Command::new("python3");
    command.arg("-I").arg("-B");
    command
}

#[test]
fn the_resolver_and_packaging_tests_pass() {
    let root = root();
    let out = python()
        .args(["-m", "unittest", "discover", "-s"])
        .arg(root.join("examples/tests/skill"))
        .args(["-p", "test_*.py"])
        .current_dir(&root)
        .output()
        .expect("python3 3.11 or newer runs the tests of the resolver and the packaging");
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "The Python tests failed:\n{log}");
    assert!(log.contains("OK"), "{log}");
}

#[test]
fn the_resolver_reports_self_in_this_repository() {
    let root = root();
    let out = python()
        .arg(root.join("skills/cicada/scripts/cicada_source.py"))
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .current_dir(&root)
        .output()
        .expect("python3 runs the resolver");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert_eq!(report["status"], "self", "{report:#}");
    assert_eq!(report["skill_version"], workspace_version(&root).as_str());
}
