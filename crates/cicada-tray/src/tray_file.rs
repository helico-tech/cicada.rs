//! The tray's own file: `tray.toml` next to the config file holds, per context, the types its worker runs and its
//! concurrency (`[contexts.<name>]` with `types` and `concurrency`). It holds no secrets.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cicada_app::ContextFile;
use serde::{Deserialize, Serialize};

/// The name of the tray's file.
pub(crate) const FILE: &str = "tray.toml";
/// The name of the lock file of the tray's file.
const LOCK: &str = "tray.toml.lock";

/// What the tray's worker runs for one context.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerSettings {
    /// The names of the selected types, of which the binary may no longer register some.
    pub types: Vec<String>,
    /// How many turns run at once.
    pub concurrency: usize,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contents {
    #[serde(default)]
    contexts: BTreeMap<String, WorkerSettings>,
}

/// The contents of the tray's file at a path.
#[derive(Debug)]
pub(crate) struct TrayFile {
    path: PathBuf,
    contents: Contents,
}

/// The tray's file for the config file `config`: `tray.toml` in the folder of the config path as given (a relative one
/// taken against the working directory; a symlink keeps it next to the link). A config path whose name, or whose
/// resolved target's name, is `tray.toml` or `tray.toml.lock` is refused, so that no write of the config file replaces
/// either.
pub(crate) fn locate(config: &ContextFile) -> Result<PathBuf, String> {
    let given = config.path();
    let resolved = config.resolved_path().map_err(|e| format!("cannot follow {}: {e}", given.display()))?;
    for path in [given, resolved.as_path()] {
        if path.file_name().is_some_and(|name| name == FILE || name == LOCK) {
            return Err(format!(
                "the config file {} is named like the tray's own {FILE}; set CICADA_CONFIG to another file",
                path.display()
            ));
        }
    }
    let absolute = if given.is_relative() {
        std::env::current_dir().map_err(|e| format!("cannot find the working directory: {e}"))?.join(given)
    } else {
        given.to_owned()
    };
    Ok(absolute.parent().unwrap_or(Path::new("/")).join(FILE))
}

impl TrayFile {
    /// Reads the file at `path`; a missing file has no entries.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let contents = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("cannot read {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Contents::default(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Ok(Self { path: path.to_owned(), contents })
    }

    /// Locks the file at `path` (an exclusive lock on `tray.toml.lock` next to it), reads it, applies `f` and saves it
    /// when `f` returns `Ok`; then unlocks.
    pub fn update<T>(path: &Path, f: impl FnOnce(&mut Self) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _lock = lock(&path.with_file_name(LOCK))?;
        let mut file = Self::load(path)?;
        let value = f(&mut file)?;
        file.save()?;
        Ok(value)
    }

    /// The saved settings of `context`; `None` when it inherits the environment and the defaults.
    pub fn get(&self, context: &str) -> Option<&WorkerSettings> {
        self.contents.contexts.get(context)
    }

    /// Sets the settings of `context`.
    pub fn set(&mut self, context: &str, settings: WorkerSettings) {
        self.contents.contexts.insert(context.to_owned(), settings);
    }

    /// Writes the whole file through a new file renamed over it, with mode 0644 on Unix.
    fn save(&self) -> anyhow::Result<()> {
        let dir = self.path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let text = toml::to_string_pretty(&self.contents)?;
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(dir)?;
            let mut file = tempfile::Builder::new().prefix(".cicada-tray").tempfile_in(dir)?;
            file.write_all(text.as_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.as_file().set_permissions(std::fs::Permissions::from_mode(0o644))?;
            }
            file.as_file().sync_all()?;
            file.persist(&self.path)?;
            Ok(())
        };
        write().with_context(|| format!("cannot write {}", self.path.display()))
    }
}

/// Takes an exclusive lock on the file `path`, creating it and its folder when missing, and waits for another holder;
/// `None` on a file system without locks.
fn lock(path: &Path) -> anyhow::Result<Option<File>> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    match file.lock() {
        Ok(()) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot lock {}", path.display())),
    }
}

/// The arguments the worker of `context` gets from its saved settings: none without an entry; else `--types` with the
/// saved names the binary still registers (`all` when that is every registered type) and `--concurrency`. A selection
/// left with no registered type, or with a lone type named `all` or `none` among others (which `--types` would read as
/// a keyword), is an error that sends the user to Settings.
pub(crate) fn worker_args(
    context: &str,
    saved: Option<&WorkerSettings>,
    registered: &[&str],
) -> Result<Vec<String>, String> {
    let Some(saved) = saved else { return Ok(Vec::new()) };
    let types: Vec<&str> = registered.iter().copied().filter(|name| saved.types.iter().any(|t| t == name)).collect();
    let types = match types.as_slice() {
        [] => return Err(format!("no registered type selected for {context}; open Settings")),
        _ if types.len() == registered.len() => "all".to_owned(),
        ["all" | "none"] => {
            return Err(format!("the saved type selection of {context} cannot be passed to --types; open Settings"));
        }
        _ => types.join(","),
    };
    Ok(vec![format!("--types={types}"), format!("--concurrency={}", saved.concurrency)])
}

/// The worker arguments of `context` from the tray's file next to the config file of `env`.
pub(crate) fn saved_args(env: &cicada_app::Env, context: &str, registered: &[&str]) -> Result<Vec<String>, String> {
    let config = ContextFile::load(env).map_err(|e| format!("{e:#}"))?;
    let tray = TrayFile::load(&locate(&config)?).map_err(|e| format!("{e:#}"))?;
    worker_args(context, tray.get(context), registered)
}

#[cfg(test)]
mod tests {
    use cicada_app::Env;

    use super::*;

    fn env(config: &Path) -> Env {
        Env::from([("CICADA_CONFIG".to_owned(), config.to_str().unwrap().to_owned())])
    }

    fn settings(types: &[&str], concurrency: usize) -> WorkerSettings {
        WorkerSettings { types: types.iter().map(|t| t.to_string()).collect(), concurrency }
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(TrayFile::load(&path).unwrap().get("prod"), None);
        TrayFile::update(&path, |file| {
            file.set("prod", settings(&["a"], 4));
            file.set("odd name.x", settings(&["a", "b"], 16));
            Ok(())
        })
        .unwrap();
        let file = TrayFile::load(&path).unwrap();
        assert_eq!(file.get("prod"), Some(&settings(&["a"], 4)));
        assert_eq!(file.get("odd name.x"), Some(&settings(&["a", "b"], 16)), "a quoted key");
        assert_eq!(file.get("test"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
        }
    }

    #[test]
    fn it_sits_next_to_the_config_path_as_given() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        assert_eq!(locate(&ContextFile::load(&env(&config)).unwrap()).unwrap(), dir.path().join(FILE));
        let relative = ContextFile::load(&env(Path::new("some/config.toml"))).unwrap();
        assert_eq!(locate(&relative).unwrap(), std::env::current_dir().unwrap().join("some").join(FILE));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_keeps_it_next_to_the_link() {
        let (links, targets) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let link = links.path().join("config.toml");
        std::os::unix::fs::symlink(targets.path().join("real.toml"), &link).unwrap();
        assert_eq!(locate(&ContextFile::load(&env(&link)).unwrap()).unwrap(), links.path().join(FILE));
    }

    #[test]
    fn a_config_named_like_the_tray_file_or_its_lock_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for name in [FILE, LOCK] {
            let error = locate(&ContextFile::load(&env(&dir.path().join(name))).unwrap()).unwrap_err();
            assert!(error.contains("CICADA_CONFIG"), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_config_that_resolves_to_the_tray_file_or_its_lock_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for (link, target) in [("config.toml", FILE), ("other.toml", LOCK)] {
            std::os::unix::fs::symlink(target, dir.path().join(link)).unwrap();
            let error = locate(&ContextFile::load(&env(&dir.path().join(link))).unwrap()).unwrap_err();
            assert!(error.contains(target), "{error}");
        }
    }

    #[test]
    fn two_config_files_in_one_folder_keep_both_entries() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.toml"), dir.path().join("b.toml"));
        let path = locate(&ContextFile::load(&env(&a)).unwrap()).unwrap();
        assert_eq!(path, locate(&ContextFile::load(&env(&b)).unwrap()).unwrap());
        std::thread::scope(|scope| {
            for context in ["of-a", "of-b"] {
                let path = &path;
                scope.spawn(move || {
                    TrayFile::update(path, |file| {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        file.set(context, settings(&["x"], 1));
                        Ok(())
                    })
                    .unwrap()
                });
            }
        });
        let file = TrayFile::load(&path).unwrap();
        assert!(file.get("of-a").is_some() && file.get("of-b").is_some());
    }

    #[test]
    fn no_entry_adds_no_arguments() {
        assert_eq!(worker_args("prod", None, &["a", "b"]), Ok(Vec::new()));
    }

    /// Scenario: Fewer types.
    #[test]
    fn an_entry_adds_the_types_and_the_concurrency() {
        assert_eq!(
            worker_args("prod", Some(&settings(&["b", "old"], 4)), &["a", "b"]).unwrap(),
            ["--types=b", "--concurrency=4"]
        );
        assert_eq!(
            worker_args("prod", Some(&settings(&["c", "a"], 2)), &["a", "b", "c"]).unwrap(),
            ["--types=a,c", "--concurrency=2"]
        );
    }

    /// Scenario: All types over the environment.
    #[test]
    fn every_registered_type_is_all() {
        let args = worker_args("prod", Some(&settings(&["b", "a", "old"], 16)), &["a", "b"]).unwrap();
        assert_eq!(args, ["--types=all", "--concurrency=16"]);
        for only in ["all", "none"] {
            let args = worker_args("prod", Some(&settings(&[only], 1)), &[only]).unwrap();
            assert_eq!(args[0], "--types=all", "a lone registered {only} runs exactly that type");
        }
    }

    /// Scenario: No registered type left.
    #[test]
    fn no_registered_type_left_fails() {
        for registered in [&["a", "b"][..], &[]] {
            let error = worker_args("prod", Some(&settings(&["old"], 1)), registered).unwrap_err();
            assert_eq!(error, "no registered type selected for prod; open Settings");
        }
    }

    /// Scenario: A type literally named all or none.
    #[test]
    fn a_lone_all_or_none_among_others_fails() {
        for only in ["all", "none"] {
            let error = worker_args("prod", Some(&settings(&[only], 1)), &[only, "b"]).unwrap_err();
            assert_eq!(error, "the saved type selection of prod cannot be passed to --types; open Settings");
        }
        let error = worker_args("prod", Some(&settings(&["all", "old"], 1)), &["all", "new"]).unwrap_err();
        assert!(error.contains("open Settings"), "saved [all, old] against [all, new] is not --types all");
    }

    #[test]
    fn saved_args_read_the_file_next_to_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        assert_eq!(saved_args(&env(&config), "prod", &["a"]), Ok(Vec::new()));
        TrayFile::update(&dir.path().join(FILE), |file| {
            file.set("prod", settings(&["a"], 3));
            Ok(())
        })
        .unwrap();
        assert_eq!(saved_args(&env(&config), "prod", &["a", "b"]).unwrap(), ["--types=a", "--concurrency=3"]);
        std::fs::write(dir.path().join(FILE), "[contexts.prod]\ntypes = 1\n").unwrap();
        assert!(saved_args(&env(&config), "prod", &["a"]).unwrap_err().contains("cannot read"));
        let error = saved_args(&env(&dir.path().join(LOCK)), "prod", &["a"]).unwrap_err();
        assert!(error.contains("CICADA_CONFIG"), "{error}");
    }
}
