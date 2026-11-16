//! Checks of the consumer skills `skills/cicada` (ADR 0041) and `skills/actor-design` (ADR 0051): their frontmatter
//! and versions, that every route resolves to a file and heading of this source, that the routes of `cicada` cover
//! every guide, reference and worked example and those of `actor-design` the design guide, that the design template
//! follows the guide's nine steps, that consumers are not routed to proposals or the change loops, the evaluation
//! cases, and the Python tests of the resolver and the packaging (`examples/tests/skill/test_*.py`), and the shape of
//! the Claude Code plugin that carries the consumer skills (ADR 0049). They check structure; whether a skill leads to
//! good decisions is judged by evaluations in review, and the plugin's manifests by `claude plugin validate` in review.

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

fn design_skill() -> String {
    read(&root().join("skills/actor-design/SKILL.md"))
}

fn design_template() -> String {
    read(&root().join("skills/actor-design/assets/design-template.md"))
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

/// The manifests of the Claude Code plugin whose source is the repository root (ADR 0049).
const MARKETPLACE: &str = ".claude-plugin/marketplace.json";
const PLUGIN: &str = ".claude-plugin/plugin.json";

/// Who owns the marketplace and writes the plugin, and where its source is.
const OWNER: &str = "helico-tech";
const REPOSITORY: &str = "https://github.com/helico-tech/cicada.rs";

/// The default component paths of a plugin other than `skills/`: the root is the plugin, so any of them there would
/// load with the consumer skills.
const FORBIDDEN_COMPONENTS: [&str; 10] = [
    "commands",
    "agents",
    "hooks",
    "bin",
    "output-styles",
    "workflows",
    "themes",
    "monitors",
    ".mcp.json",
    ".lsp.json",
];

/// What a path is, without following a symbolic link at its last component.
#[derive(Debug, PartialEq)]
enum Kind {
    Missing,
    Link,
    Dir,
    File,
    Other,
}

fn kind(path: &Path) -> Kind {
    match fs::symlink_metadata(path) {
        Err(_) => Kind::Missing,
        Ok(m) if m.file_type().is_symlink() => Kind::Link,
        Ok(m) if m.is_dir() => Kind::Dir,
        Ok(m) if m.is_file() => Kind::File,
        Ok(_) => Kind::Other,
    }
}

type Object = serde_json::Map<String, serde_json::Value>;

/// The JSON object of the manifest at `path`, or nothing with the problem added.
fn manifest(root: &Path, path: &str, problems: &mut Vec<String>) -> Option<Object> {
    match kind(&root.join(path)) {
        Kind::File => {}
        Kind::Missing => {
            problems.push(format!("{path} is missing"));
            return None;
        }
        other => {
            problems.push(format!("{path} is not a regular file but {other:?}"));
            return None;
        }
    }
    match serde_json::from_str(&read(&root.join(path))) {
        Ok(serde_json::Value::Object(object)) => Some(object),
        Ok(_) => {
            problems.push(format!("{path} is not a JSON object"));
            None
        }
        Err(e) => {
            problems.push(format!("{path} is not JSON: {e}"));
            None
        }
    }
}

/// Adds a problem for each field of `object` (at `at`) that is not one of `keys`, and each of `keys` it lacks.
fn exact_keys(path: &str, at: &str, object: &Object, keys: &[&str], problems: &mut Vec<String>) {
    for key in object.keys().filter(|k| !keys.contains(&k.as_str())) {
        problems.push(format!("{path}: the field {at}{key} is not allowed"));
    }
    for key in keys.iter().filter(|k| !object.contains_key(**k)) {
        problems.push(format!("{path}: the field {at}{key} is missing"));
    }
}

/// Adds a problem when the field `at` of `object` is present and not `want`.
fn expect(
    path: &str,
    at: &str,
    value: Option<&serde_json::Value>,
    want: serde_json::Value,
    problems: &mut Vec<String>,
) {
    if let Some(value) = value.filter(|v| **v != want) {
        problems.push(format!("{path}: the field {at} must be {want}, is {value}"));
    }
}

/// Adds a problem when the field `at` is present and not a nonempty string.
fn text_field(path: &str, at: &str, value: Option<&serde_json::Value>, problems: &mut Vec<String>) {
    if value.is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty())) {
        problems.push(format!("{path}: the field {at} must be a nonempty string"));
    }
}

/// What is wrong with the plugin at `root`: its two manifests against the shape of ADR 0049 and the workspace version,
/// a skill folder that is not a real folder with a regular `SKILL.md`, a symbolic link, or another plugin component.
fn plugin_problems(root: &Path) -> Vec<String> {
    use serde_json::json;
    let mut problems = Vec::new();
    if kind(&root.join(".claude-plugin")) == Kind::Link {
        problems.push(".claude-plugin is a symbolic link".to_owned());
    }
    if let Some(m) = manifest(root, MARKETPLACE, &mut problems) {
        let p = MARKETPLACE;
        exact_keys(p, "", &m, &["name", "owner", "description", "plugins"], &mut problems);
        expect(p, "name", m.get("name"), json!("cicada"), &mut problems);
        expect(p, "owner", m.get("owner"), json!({ "name": OWNER }), &mut problems);
        text_field(p, "description", m.get("description"), &mut problems);
        match m.get("plugins").map(|v| v.as_array()) {
            None => {}
            Some(Some(plugins)) if plugins.len() == 1 => match plugins[0].as_object() {
                Some(entry) => {
                    exact_keys(p, "plugins[0].", entry, &["name", "source"], &mut problems);
                    expect(p, "plugins[0].name", entry.get("name"), json!("cicada"), &mut problems);
                    expect(p, "plugins[0].source", entry.get("source"), json!("./"), &mut problems);
                }
                None => problems.push(format!("{p}: the field plugins[0] must be an object")),
            },
            Some(_) => problems.push(format!("{p}: the field plugins must be a list of exactly one plugin")),
        }
    }
    if let Some(m) = manifest(root, PLUGIN, &mut problems) {
        let p = PLUGIN;
        exact_keys(p, "", &m, &["name", "version", "description", "author", "repository"], &mut problems);
        expect(p, "name", m.get("name"), json!("cicada"), &mut problems);
        match m.get("version").map(|v| v.as_str()) {
            None => {}
            Some(None) => problems.push(format!("{p}: the field version must be a string")),
            Some(Some(version)) => {
                let workspace = workspace_version(root);
                if version != workspace {
                    problems
                        .push(format!("{p}: the field version is {version}, but the workspace version is {workspace}"));
                }
            }
        }
        text_field(p, "description", m.get("description"), &mut problems);
        expect(p, "author", m.get("author"), json!({ "name": OWNER }), &mut problems);
        expect(p, "repository", m.get("repository"), json!(REPOSITORY), &mut problems);
    }
    problems.extend(skills_problems(root));
    for component in FORBIDDEN_COMPONENTS {
        if kind(&root.join(component)) != Kind::Missing {
            problems.push(format!("{component} at the root would load as a component of the plugin"));
        }
    }
    problems
}

/// What is wrong with `skills/` at `root`: each entry is a real folder with a regular `SKILL.md`, the tree holds no
/// symbolic link, and the skill `cicada` is there.
fn skills_problems(root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let skills = root.join("skills");
    if kind(&skills) != Kind::Dir {
        return vec![format!("skills is not a real folder but {:?}", kind(&skills))];
    }
    let mut names: Vec<String> = fs::read_dir(&skills)
        .expect("skills/")
        .map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".DS_Store") // ignored by git, so never in the plugin
        .collect();
    names.sort();
    for name in &names {
        match kind(&skills.join(name)) {
            Kind::Dir => match kind(&skills.join(name).join("SKILL.md")) {
                Kind::File | Kind::Link => {} // a link is named below
                other => problems.push(format!("skills/{name}/SKILL.md is not a regular file but {other:?}")),
            },
            Kind::Link => {} // named below
            other => problems.push(format!("skills/{name} is not a skill folder but {other:?}")),
        }
    }
    let mut folders = vec![skills];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).expect("a folder of skills/") {
            let path = entry.expect("an entry").path();
            match kind(&path) {
                Kind::Link => problems
                    .push(format!("{} is a symbolic link", path.strip_prefix(root).expect("under the root").display())),
                Kind::Dir => folders.push(path),
                _ => {}
            }
        }
    }
    if !names.iter().any(|n| n == "cicada") {
        problems.push("skills/cicada is missing".to_owned());
    }
    problems
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
const REQUIRED_ROUTES: [&str; 25] = [
    "README.md#using-a-release-in-another-project",
    "docs/guide/design.md#from-problem-to-design",
    "docs/guide/design.md#choose-actors-and-identities",
    "docs/guide/design.md#design-protocols",
    "docs/guide/design.md#coordinate-work-and-time",
    "docs/guide/design.md#recover-from-failure",
    "docs/guide/design.md#work-with-outside-systems",
    "docs/guide/design.md#choose-a-pattern",
    "docs/guide/design.md#recognize-design-mistakes",
    "docs/guide/design.md#a-websocket-session",
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
    for (name, text) in [("cicada", skill()), ("actor-design", design_skill())] {
        let (top, metadata) = frontmatter(&text);
        let keys: Vec<&str> = top.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["name", "description", "metadata"], "{name}: only these keys, no harness metadata");
        assert_eq!(top[0].1, name);
        let description = &top[1].1;
        assert!(
            description.len() > 100 && description.len() <= 1024,
            "{name}: a description of at most 1024 characters"
        );
        assert_eq!(metadata.len(), 1, "{name}: metadata holds the version only: {metadata:?}");
        assert_eq!(metadata[0].0, "cicada-version");
        assert_eq!(
            metadata[0].1,
            workspace_version(&root()),
            "{name}: metadata.cicada-version is the workspace version"
        );
    }
}

#[test]
fn the_plugin_carries_the_consumer_skills_and_nothing_else() {
    let problems = plugin_problems(&root());
    assert!(problems.is_empty(), "The plugin:\n  {}", problems.join("\n  "));
}

/// A good plugin of workspace version 1.2.3 in a temporary folder.
fn plugin_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("Cargo.toml"), "[workspace.package]\nversion = \"1.2.3\"\n").unwrap();
    fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    edit_json(root, MARKETPLACE, |m| {
        *m = serde_json::json!({ "name": "cicada", "owner": { "name": OWNER }, "description": "Skills.",
                                 "plugins": [{ "name": "cicada", "source": "./" }] })
    });
    edit_json(root, PLUGIN, |m| {
        *m = serde_json::json!({ "name": "cicada", "version": "1.2.3", "description": "Skills.",
                                 "author": { "name": OWNER }, "repository": REPOSITORY })
    });
    fs::create_dir_all(root.join("skills/cicada/scripts")).unwrap();
    fs::write(root.join("skills/cicada/SKILL.md"), "---\nname: cicada\n---\n").unwrap();
    fs::write(root.join("skills/cicada/scripts/cicada_source.py"), "\n").unwrap();
    dir
}

/// Rewrites the JSON file at `path` under `root` (`null` when it does not exist) through `change`.
fn edit_json(root: &Path, path: &str, change: impl FnOnce(&mut serde_json::Value)) {
    let file = root.join(path);
    let mut value = fs::read_to_string(&file).map_or(serde_json::Value::Null, |t| serde_json::from_str(&t).unwrap());
    change(&mut value);
    fs::write(file, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

#[cfg(unix)]
fn link(target: &str, at: &Path) {
    std::os::unix::fs::symlink(target, at).unwrap();
}

/// A change to a good plugin, with the problem it must cause.
#[cfg(unix)]
type Case = (&'static str, Box<dyn Fn(&Path)>, &'static str);

/// Scenario: An unintended component or alternate source; and a stale or invalid manifest.
#[cfg(unix)]
#[test]
fn a_changed_manifest_another_component_or_a_link_is_named() {
    let ok = plugin_fixture();
    assert_eq!(plugin_problems(ok.path()), Vec::<String>::new(), "the fixture is a good plugin");
    fs::create_dir_all(ok.path().join("skills/actor-design")).unwrap();
    fs::write(ok.path().join("skills/actor-design/SKILL.md"), "---\nname: actor-design\n---\n").unwrap();
    fs::write(ok.path().join("skills/.DS_Store"), "").unwrap();
    assert_eq!(plugin_problems(ok.path()), Vec::<String>::new(), "a second ordinary skill is accepted");

    let json = |path: &'static str, change: fn(&mut serde_json::Value)| -> Box<dyn Fn(&Path)> {
        Box::new(move |root| edit_json(root, path, change))
    };
    let write = |path: &'static str, text: &'static str| -> Box<dyn Fn(&Path)> {
        Box::new(move |root| fs::write(root.join(path), text).unwrap())
    };
    let remove =
        |path: &'static str| -> Box<dyn Fn(&Path)> { Box::new(move |root| fs::remove_file(root.join(path)).unwrap()) };
    let cases: Vec<Case> = vec![
        ("no marketplace", remove(MARKETPLACE), "marketplace.json is missing"),
        ("no plugin", remove(PLUGIN), "plugin.json is missing"),
        ("bad marketplace JSON", write(MARKETPLACE, "{"), "marketplace.json is not JSON"),
        ("bad plugin JSON", write(PLUGIN, "{\"name\": "), "plugin.json is not JSON"),
        ("plugin list", write(PLUGIN, "[]"), "plugin.json is not a JSON object"),
        ("no version", json(PLUGIN, |m| _ = m.as_object_mut().unwrap().remove("version")), "field version is missing"),
        ("number version", json(PLUGIN, |m| m["version"] = 1.into()), "field version must be a string"),
        (
            "stale version",
            json(PLUGIN, |m| m["version"] = "1.2.2".into()),
            "version is 1.2.2, but the workspace version is 1.2.3",
        ),
        ("plugin name", json(PLUGIN, |m| m["name"] = "other".into()), "plugin.json: the field name must be"),
        ("blank description", json(PLUGIN, |m| m["description"] = " ".into()), "description must be a nonempty"),
        ("author text", json(PLUGIN, |m| m["author"] = OWNER.into()), "the field author must be"),
        ("repository", json(PLUGIN, |m| m["repository"] = "https://example.com".into()), "field repository must be"),
        ("skills path", json(PLUGIN, |m| m["skills"] = "./more".into()), "the field skills is not allowed"),
        ("inline hooks", json(PLUGIN, |m| m["hooks"] = serde_json::json!({})), "the field hooks is not allowed"),
        ("marketplace name", json(MARKETPLACE, |m| m["name"] = "x".into()), "marketplace.json: the field name must"),
        ("owner", json(MARKETPLACE, |m| m["owner"]["email"] = "a@b".into()), "the field owner must be"),
        ("version pin", json(MARKETPLACE, |m| m["version"] = "1.2.3".into()), "field version is not allowed"),
        (
            "second plugin",
            json(MARKETPLACE, |m| m["plugins"].as_array_mut().unwrap().push(serde_json::json!({ "name": "b" }))),
            "plugins must be a list of exactly one plugin",
        ),
        ("entry name", json(MARKETPLACE, |m| m["plugins"][0]["name"] = "x".into()), "plugins[0].name must be"),
        ("sub folder", json(MARKETPLACE, |m| m["plugins"][0]["source"] = "./plugin".into()), "plugins[0].source"),
        (
            "remote source",
            json(MARKETPLACE, |m| {
                m["plugins"][0]["source"] = serde_json::json!({ "source": "github", "repo": "a/b", "ref": "v1.2.3" })
            }),
            "plugins[0].source must be",
        ),
        (
            "entry commands",
            json(MARKETPLACE, |m| m["plugins"][0]["commands"] = serde_json::json!(["./x"])),
            "plugins[0].commands is not allowed",
        ),
        ("entry version", json(MARKETPLACE, |m| m["plugins"][0]["version"] = "1".into()), "[0].version is not allowed"),
        ("skills file", write("skills/README.md", "x"), "skills/README.md is not a skill folder"),
        (
            "no SKILL.md",
            Box::new(|root| fs::create_dir(root.join("skills/empty")).unwrap()),
            "skills/empty/SKILL.md is not a regular file but Missing",
        ),
        (
            "no cicada",
            Box::new(|root| fs::remove_dir_all(root.join("skills/cicada")).unwrap()),
            "skills/cicada is missing",
        ),
        (
            "linked skill",
            Box::new(|root| link("cicada", &root.join("skills/other"))),
            "skills/other is a symbolic link",
        ),
        (
            "linked file",
            Box::new(|root| link("../../../Cargo.toml", &root.join("skills/cicada/scripts/x"))),
            "skills/cicada/scripts/x is a symbolic link",
        ),
        (
            "linked SKILL.md",
            Box::new(|root| {
                fs::rename(root.join("skills/cicada/SKILL.md"), root.join("skill.md")).unwrap();
                link("../../skill.md", &root.join("skills/cicada/SKILL.md"));
            }),
            "skills/cicada/SKILL.md is a symbolic link",
        ),
        (
            "linked manifest",
            Box::new(|root| {
                fs::rename(root.join(PLUGIN), root.join("plugin.json")).unwrap();
                link("../plugin.json", &root.join(PLUGIN));
            }),
            "plugin.json is not a regular file but Link",
        ),
        (
            "linked manifest folder",
            Box::new(|root| {
                fs::rename(root.join(".claude-plugin"), root.join("manifests")).unwrap();
                link("manifests", &root.join(".claude-plugin"));
            }),
            ".claude-plugin is a symbolic link",
        ),
        (
            "linked skills folder",
            Box::new(|root| {
                fs::rename(root.join("skills"), root.join("real")).unwrap();
                link("real", &root.join("skills"));
            }),
            "skills is not a real folder but Link",
        ),
    ];
    for (name, change, want) in cases {
        let dir = plugin_fixture();
        change(dir.path());
        let problems = plugin_problems(dir.path());
        assert!(problems.iter().any(|p| p.contains(want)), "{name}: expected {want:?} in {problems:#?}");
    }
    for component in FORBIDDEN_COMPONENTS {
        for linked in [false, true] {
            let dir = plugin_fixture();
            let at = dir.path().join(component);
            match (linked, component.ends_with(".json")) {
                (true, _) => link("nowhere", &at),
                (false, true) => fs::write(&at, "{}").unwrap(),
                (false, false) => fs::create_dir(&at).unwrap(),
            }
            let problems = plugin_problems(dir.path());
            let want = format!("{component} at the root would load as a component of the plugin");
            assert!(problems.contains(&want), "{component}, linked {linked}: {problems:#?}");
        }
    }
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

/// The heading of the section of `actor-design` whose code spans are routes into the resolved source. Paths elsewhere
/// in that skill, such as the saved document `docs/design/actors.md` or its template, are not routes.
const SOURCE_ROUTES: &str = "## Source routes";

/// The text of the `##` section headed `heading`, up to the next `##` heading; empty when there is none.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let Some(start) = text.lines().position(|l| l == heading) else { return "" };
    let offset: usize = text.lines().take(start + 1).map(|l| l.len() + 1).sum();
    let rest = text.get(offset..).unwrap_or_default();
    let end = rest.find("\n## ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

/// The routes `actor-design` must name: every section of the design guide, and the short rules of one actor type that
/// an older source without the guide falls back to.
const REQUIRED_DESIGN_ROUTES: [&str; 10] = [
    "docs/guide/design.md#from-problem-to-design",
    "docs/guide/design.md#choose-actors-and-identities",
    "docs/guide/design.md#design-protocols",
    "docs/guide/design.md#coordinate-work-and-time",
    "docs/guide/design.md#recover-from-failure",
    "docs/guide/design.md#work-with-outside-systems",
    "docs/guide/design.md#choose-a-pattern",
    "docs/guide/design.md#recognize-design-mistakes",
    "docs/guide/design.md#a-websocket-session",
    "docs/guide/core.md#design-an-actor",
];

/// The first cells of the table rows of `text` that start with a step number, such as `1. The promise`, in order.
fn numbered_cells(text: &str) -> Vec<&str> {
    text.lines().filter_map(|line| line.strip_prefix("| ")?.split(" |").next()).filter(|cell| numbered(cell)).collect()
}

/// Whether `text` starts with a step number and a period, as `3. Who owns each rule`.
fn numbered(text: &str) -> bool {
    text.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok())
}

/// The nine steps of the guide's table under From problem to design, in order.
fn guide_steps(guide: &str) -> Vec<String> {
    numbered_cells(section(guide, "## From problem to design")).into_iter().map(str::to_owned).collect()
}

/// The sections the saved design document has besides one per step.
const TEMPLATE_SECTIONS: [&str; 4] = ["Source and scope", "Progress", "Decisions", "Open questions"];

/// What is wrong with a design template against the guide's `steps`: a missing section, step headings that are not
/// exactly the steps in order, or a Progress table that does not list them in order.
fn template_problems(template: &str, steps: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let headings: Vec<&str> = template.lines().filter_map(|l| l.strip_prefix("## ")).collect();
    for want in TEMPLATE_SECTIONS {
        if !headings.contains(&want) {
            problems.push(format!("the template has no section {want}"));
        }
    }
    let sections: Vec<&str> = headings.iter().copied().filter(|h| numbered(h)).collect();
    if sections != steps {
        problems.push(format!("the template's step sections are {sections:?}, not the guide's {steps:?}"));
    }
    let progress = numbered_cells(section(template, "## Progress"));
    if progress != steps {
        problems.push(format!("the template's Progress lists {progress:?}, not the guide's {steps:?}"));
    }
    problems
}

/// What the method of `actor-design` misses of the guide's `steps`: each named in order, whitespace aside.
fn method_problems(skill: &str, steps: &[String]) -> Vec<String> {
    let flat = skill.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut from = 0;
    let mut problems = Vec::new();
    for step in steps {
        match flat[from..].find(step.as_str()) {
            Some(at) => from += at + step.len(),
            None => problems.push(format!("SKILL.md does not name the step {step} in the guide's order")),
        }
    }
    problems
}

#[test]
fn the_design_skill_routes_resolve_and_cover_the_design_guide() {
    let text = design_skill();
    let routes_section = section(&text, SOURCE_ROUTES);
    assert!(routes(routes_section).len() >= REQUIRED_DESIGN_ROUTES.len(), "the Source routes section is found");
    let problems = check_routes(routes_section, &root());
    assert!(problems.is_empty(), "Routes that do not resolve:\n  {}", problems.join("\n  "));
    let required: Vec<String> = REQUIRED_DESIGN_ROUTES.map(str::to_owned).into();
    let problems = missing_coverage(routes_section, &required);
    assert!(problems.is_empty(), "actor-design misses:\n  {}", problems.join("\n  "));
}

#[test]
fn the_design_skill_names_no_contributor_material() {
    for (name, text) in [("SKILL.md", design_skill()), ("design-template.md", design_template())] {
        assert!(!text.contains(SELF_SECTION), "{name} has no section for working on Cicada itself");
        let problems = contributor_routes(&text);
        assert!(problems.is_empty(), "{name}:\n{}", problems.join("\n"));
    }
}

#[test]
fn the_design_template_and_method_follow_the_guides_nine_steps() {
    let steps = guide_steps(&read(&root().join("docs/guide/design.md")));
    assert_eq!(steps.len(), 9, "the guide's table has nine steps: {steps:?}");
    let problems = template_problems(&design_template(), &steps);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let problems = method_problems(&design_skill(), &steps);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_two_skills_point_to_each_other_by_name() {
    assert!(skill().contains("`actor-design`"), "cicada names actor-design for design work");
    assert!(design_skill().contains("`cicada`"), "actor-design names cicada for building and running");
}

/// Scenario: Route and template checks.
#[test]
fn a_missing_design_route_step_or_section_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("docs/guide")).unwrap();
    fs::write(root.join("docs/guide/design.md"), "# Design\n\n## From problem to design\n").unwrap();
    let text = "Save to `docs/design/actors.md`.\n\n## Source routes\n\n`docs/guide/design.md#from-problem-to-design`, \
                `docs/guide/design.md#gone`\n\n## Later\n\n`docs/elsewhere.md`\n";
    let routes_section = section(text, SOURCE_ROUTES);
    assert_eq!(
        check_routes(routes_section, root),
        ["SKILL.md:2: the route docs/guide/design.md#gone names an anchor that no heading has"],
        "only the Source routes section holds routes; the saved document's path is not one"
    );
    assert_eq!(
        missing_coverage(routes_section, &["docs/guide/core.md#design-an-actor".to_owned()]),
        ["SKILL.md routes nothing to docs/guide/core.md#design-an-actor"]
    );
    let guide = "## From problem to design\n\n| Step | Ask |\n| --- | --- |\n| 1. One | a |\n| 2. Two | b |\n\n## Next\n\n\
                 | 3. Three | c |\n";
    let steps = guide_steps(guide);
    assert_eq!(steps, ["1. One", "2. Two"]);
    let template = "## Source and scope\n\n## Progress\n\n| 2. Two | draft |\n| 1. One | draft |\n\n## 1. One\n\n\
                    ## Decisions\n";
    assert_eq!(
        template_problems(template, &steps),
        [
            "the template has no section Open questions",
            "the template's step sections are [\"1. One\"], not the guide's [\"1. One\", \"2. Two\"]",
            "the template's Progress lists [\"2. Two\", \"1. One\"], not the guide's [\"1. One\", \"2. Two\"]",
        ]
    );
    assert_eq!(
        method_problems("Steps: 2. Two, then 1. One.", &steps),
        ["SKILL.md does not name the step 2. Two in the guide's order"]
    );
    let text = "Read `docs/maintaining/maintenance.md`.\n";
    assert_eq!(contributor_routes(text), ["SKILL.md:1: a consumer is routed to docs/maintaining/maintenance.md"]);
}

/// The ids of the staged cases of `actor-design` that the design names.
const DESIGN_EVALUATIONS: [&str; 16] = [
    "fresh-design",
    "not-an-actor",
    "timeout-story",
    "confirmation-required",
    "delegated-decisions",
    "delegated-no-actors",
    "delegated-revision",
    "resume-changed-requirement",
    "existing-file-protection",
    "no-repository",
    "old-source",
    "mixed-source",
    "missing-sibling",
    "mismatched-sibling",
    "plugin-layout",
    "archive-layout",
];

#[test]
fn the_design_evaluation_cases_are_complete() {
    let text = read(&root().join("examples/tests/skill/actor-design-evals.json"));
    let evals: serde_json::Value = serde_json::from_str(&text).expect("actor-design-evals.json is JSON");
    assert!(evals["about"].as_str().is_some_and(|s| !s.is_empty()), "the file says how the cases are used");
    let cases = evals["cases"].as_array().expect("a list of cases");
    let ids: Vec<&str> = cases.iter().filter_map(|c| c["id"].as_str()).collect();
    assert_eq!(ids.len(), cases.len(), "every case has an id");
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "ids are unique: {ids:?}");
    for id in DESIGN_EVALUATIONS {
        assert!(ids.contains(&id), "actor-design-evals.json has no case {id}");
    }
    let texts = |value: &serde_json::Value| {
        value.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|s| s.as_str().is_some_and(|s| !s.is_empty())))
    };
    for case in cases {
        assert!(case["setup"].as_str().is_some_and(|s| !s.is_empty()), "{}: no setup", case["id"]);
        for field in ["turns", "expect", "avoid"] {
            assert!(texts(&case[field]), "{}: {field} is not a list of texts", case["id"]);
        }
    }
}
