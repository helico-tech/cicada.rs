//! The config file and which server a command talks to.
//!
//! The file holds named contexts (`[contexts.<name>]`: `server`, optional `token` and `headers`) and the
//! `current-context`. A command's target comes from its flags, else the environment, else the file; [`resolve`] is a
//! pure function of the three, so it is tested without touching the real environment.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use cicada_client::ActorClient;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::Env;

/// The names and current selection of the app's configured contexts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Contexts {
    names: Vec<String>,
    current: Option<String>,
}

impl Contexts {
    /// Loads the config file at the path resolved from `env`. A missing file has no contexts.
    pub fn load(env: &Env) -> anyhow::Result<Self> {
        let Config { contexts, current_context } = Config::load(&path(env)?)?;
        Ok(Self { names: contexts.into_keys().collect(), current: current_context })
    }

    /// Returns the configured context names in alphabetical order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Returns the current context named by the config file, if set.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }
}

/// A context of the config file: a server with its credentials. Its `Debug` shows the token and the values of the
/// headers as `***`, and the password of the server URL as `***`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEntry {
    /// Base URL of the API, e.g. `http://localhost:8080`.
    pub server: String,
    /// Sent as `Authorization: Bearer <token>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Sent with every request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl std::fmt::Debug for ContextEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextEntry")
            .field("server", &redacted_url(&self.server))
            .field("token", &self.token.as_ref().map(|_| "***"))
            .field("headers", &self.headers.keys().map(|name| (name, "***")).collect::<BTreeMap<_, _>>())
            .finish()
    }
}

/// The config file that `cicada config` reads and writes, with the same path rules, parser and atomic save (mode 0600).
///
/// A parse error names the file and the line, never the text, which can hold a token. Writers that may run next to
/// each other go through [`Self::update`], which holds an exclusive lock while it reads, changes and saves the file;
/// readers take no lock. Its `Debug` hides the credentials as [`ContextEntry`] does.
#[derive(Debug, Clone)]
pub struct ContextFile {
    path: PathBuf,
    config: Config,
    changed: bool,
}

impl ContextFile {
    /// Reads the config file at the path resolved from `env`: `$CICADA_CONFIG`, else
    /// `$XDG_CONFIG_HOME/cicada/config.toml`, else `$HOME/.config/cicada/config.toml`. A missing file has no contexts.
    pub fn load(env: &Env) -> anyhow::Result<Self> {
        let path = path(env)?;
        let config = Config::load(&path)?;
        Ok(Self { path, config, changed: false })
    }

    /// Locks the config file, reads it again, applies `f` and, when `f` returns `Ok` and changed the file since it
    /// last saved, saves it; then unlocks. Every writer that uses it, `cicada config use-context` and `set-context`
    /// among them, keeps the others' changes. The lock is an exclusive OS lock on `<target>.lock` next to the file a
    /// save writes (a symlink's target, so a link and its target share one lock), held on this machine only; on a file
    /// system without locks the update runs unlocked. `f` may call [`Self::save`] itself and keep the lock for more
    /// work.
    pub fn update<T>(env: &Env, f: impl FnOnce(&mut Self) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let path = path(env)?;
        let _lock = lock_next_to(&link_target(&path).with_context(|| format!("cannot follow {}", path.display()))?)?;
        let mut file = Self { config: Config::load(&path)?, path, changed: false };
        let value = f(&mut file)?;
        if file.changed {
            file.save()?;
        }
        Ok(value)
    }

    /// The path of the config file, as `env` gives it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file a save writes: [`Self::path`] after following its symlinks, each relative one from the folder of its
    /// link. It need not exist.
    pub fn resolved_path(&self) -> std::io::Result<PathBuf> {
        link_target(&self.path)
    }

    /// The names of the contexts, in alphabetical order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.config.contexts.keys().map(String::as_str)
    }

    /// The `current-context` of the file, if set; it need not name a context of the file.
    pub fn current(&self) -> Option<&str> {
        self.config.current_context.as_deref()
    }

    /// The context `name`, if the file has it.
    pub fn context(&self, name: &str) -> Option<ContextEntry> {
        self.config.contexts.get(name).cloned()
    }

    /// Replaces the whole context `name`, or adds it, as `cicada config set-context` does: it becomes the current
    /// context when none is.
    pub fn set_context(&mut self, name: &str, entry: ContextEntry) {
        self.config.contexts.insert(name.to_owned(), entry);
        self.config.current_context.get_or_insert_with(|| name.to_owned());
        self.changed = true;
    }

    /// Sets the server and token of context `name`, keeping its headers, or adds it without headers. Unlike
    /// [`Self::set_context`], it leaves `current-context` alone.
    pub fn set_server_and_token(&mut self, name: &str, server: String, token: Option<String>) {
        let entry = self.config.contexts.entry(name.to_owned()).or_default();
        entry.server = server;
        entry.token = token;
        self.changed = true;
    }

    /// Makes `name` the current context, as `cicada config use-context` does; an error when the file has no such
    /// context.
    pub fn use_context(&mut self, name: &str) -> anyhow::Result<()> {
        if !self.config.contexts.contains_key(name) {
            bail!("no context '{name}' in {}", self.path.display());
        }
        self.config.current_context = Some(name.to_owned());
        self.changed = true;
        Ok(())
    }

    /// Writes the whole file atomically, with mode 0600 on Unix; comments in an existing file are not kept. Use it
    /// within [`Self::update`] when another writer may run.
    pub fn save(&mut self) -> anyhow::Result<()> {
        self.config.save(&self.path)?;
        self.changed = false;
        Ok(())
    }
}

/// Takes an exclusive lock on `<file>.lock`, creating it and its folder when missing, and waits for another holder.
/// `None` on a file system without locks; the lock ends when the returned file is dropped.
pub(crate) fn lock_next_to(file: &Path) -> anyhow::Result<Option<File>> {
    let mut name = file.file_name().map(ToOwned::to_owned).unwrap_or_default();
    name.push(".lock");
    let lock = file.with_file_name(name);
    let dir = lock.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let handle = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock)
        .with_context(|| format!("cannot open {}", lock.display()))?;
    match handle.lock() {
        Ok(()) => Ok(Some(handle)),
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
            tracing::warn!("{}: no lock, as the file system gave: {e}", lock.display());
            Ok(None)
        }
        Err(e) => Err(e).with_context(|| format!("cannot lock {}", lock.display())),
    }
}

/// The config file. Its `Debug` hides the credentials as [`ContextEntry`] does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    /// The context used when neither `--context` nor `CICADA_CONTEXT` names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_context: Option<String>,
    /// The contexts, by name.
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}

/// A named server with its credentials.
pub type Context = ContextEntry;

impl Config {
    /// Reads the file at `path`; a missing file is an empty config. A parse error names the line but not the text, as
    /// the diagnostic of the parser can quote a token or server from the file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|error| {
                let before = |at: usize| text.get(..at).map_or(0, |text| text.matches('\n').count());
                let line = error.span().map(|span| before(span.start) + 1);
                let at = line.map(|line| format!(" at line {line}")).unwrap_or_default();
                anyhow!("cannot read {}: not a valid config file{at}", path.display())
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// Writes the whole file to `path`, creating its directory. Comments in an existing file are not kept.
    ///
    /// The text goes to a new file in the same directory, which is then renamed over `path`: the old file stays whole
    /// until the new one is complete, and a reader that has the old one open never sees the new text. The new file is
    /// created with mode 0600 on Unix (`tempfile`'s default), because it holds tokens, and is removed when the save
    /// fails. When `path` is a symlink, all of this happens to the file it points to, which is created when it is not
    /// there yet, and the link stays a link.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let target = link_target(path).with_context(|| format!("cannot follow {}", path.display()))?;
        let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let text = toml::to_string_pretty(self)?;
        let write = || -> std::io::Result<()> {
            let mut file = tempfile::Builder::new().prefix(".cicada-config").tempfile_in(dir)?;
            file.write_all(text.as_bytes())?;
            file.as_file().sync_all()?;
            file.persist(&target)?;
            Ok(())
        };
        write().with_context(|| format!("cannot write {}", target.display()))
    }
}

/// The file that `path` names after following its symlinks, each relative one from the folder of its link; `path`
/// itself when it is no link. The last target need not exist.
fn link_target(path: &Path) -> std::io::Result<PathBuf> {
    let mut path = path.to_path_buf();
    // As many links as Linux follows before it gives up (ELOOP).
    for _ in 0..40 {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = std::fs::read_link(&path)?;
                path = match path.parent() {
                    Some(dir) if link.is_relative() => dir.join(link),
                    _ => link,
                };
            }
            _ => return Ok(path),
        }
    }
    Err(std::io::Error::other("too many levels of symbolic links"))
}

/// The config file's path: `$CICADA_CONFIG`, else `$XDG_CONFIG_HOME/cicada/config.toml`, else
/// `$HOME/.config/cicada/config.toml`.
pub fn path(env: &Env) -> anyhow::Result<PathBuf> {
    let set = |name: &str| env.get(name).filter(|v| !v.is_empty());
    if let Some(file) = set("CICADA_CONFIG") {
        return Ok(PathBuf::from(file));
    }
    let base = match (set("XDG_CONFIG_HOME"), set("HOME")) {
        (Some(xdg), _) => PathBuf::from(xdg),
        (None, Some(home)) => Path::new(home).join(".config"),
        (None, None) => bail!("cannot find the config file: set CICADA_CONFIG or HOME"),
    };
    Ok(base.join("cicada").join("config.toml"))
}

/// What the global flags say about the target.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    /// `--context`.
    pub context: Option<String>,
    /// `--server`.
    pub server: Option<String>,
    /// `--token`.
    pub token: Option<String>,
}

/// The server a command talks to, and how. Its `Debug` leaves out the token and the values of the headers, and hides the
/// password of the URL.
#[derive(Clone, PartialEq)]
pub struct Target {
    /// The context it came from, if any.
    pub context: Option<String>,
    /// Base URL of the API.
    pub server: String,
    /// Bearer token.
    pub token: Option<String>,
    /// Extra headers.
    pub headers: BTreeMap<String, String>,
}

/// Resolves the target: each of server, context and token from the flag, else the environment, else the file. `bin` is
/// the binary as it was called, for the hint.
pub fn resolve(flags: &Overrides, env: &Env, config: &Config, bin: &str) -> anyhow::Result<Target> {
    let from_env = |name: &str| env.get(name).filter(|v| !v.is_empty()).cloned();
    let name = flags.context.clone().or_else(|| from_env("CICADA_CONTEXT")).or_else(|| config.current_context.clone());
    let context = match &name {
        Some(name) => Some(config.contexts.get(name).ok_or_else(|| anyhow!("no context '{name}' in the config file"))?),
        None => None,
    };
    let server =
        flags.server.clone().or_else(|| from_env("CICADA_SERVER")).or_else(|| context.map(|c| c.server.clone()));
    let Some(server) = server else {
        bail!("no server: pass --server, set CICADA_SERVER, or add a context with `{bin} config set-context`");
    };
    Ok(Target {
        context: name,
        server,
        token: flags
            .token
            .clone()
            .or_else(|| from_env("CICADA_TOKEN"))
            .or_else(|| context.and_then(|c| c.token.clone())),
        headers: context.map(|c| c.headers.clone()).unwrap_or_default(),
    })
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("context", &self.context)
            .field("server", &redacted_url(&self.server))
            .field("token", &self.token.as_ref().map(|_| "***"))
            .field("headers", &self.headers.keys().map(|name| (name, "***")).collect::<BTreeMap<_, _>>())
            .finish()
    }
}

/// `url` with its password, if any, as `***`, read as a URL rather than scanned as text, so that no character of a
/// password ends it early. A value that is not a URL with a host (`user:pw@host` reads as the scheme `user`) keeps
/// only its scheme: `scheme://***`, else `***`.
fn redacted_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut parsed) if parsed.has_host() => {
            if parsed.password().is_some() {
                // A URL that has a password has an authority, which takes a password.
                let _ = parsed.set_password(Some("***"));
            }
            parsed.to_string()
        }
        _ => match url.split_once("://") {
            Some((scheme, _)) if !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric()) => {
                format!("{scheme}://***")
            }
            _ => "***".to_owned(),
        },
    }
}

/// How long a request of a worker over HTTP may take in all; a lease, a renewal, a release and a completion are bounded
/// by the turn timeout of [`ActorClient`] (10 s) as well.
pub(crate) const WORKER_TIMEOUT: Duration = Duration::from_secs(30);

/// Checks a server URL as a worker over HTTP does: it parses as a URL and uses `http` or `https`. The error does not
/// quote the URL.
pub fn check_server_url(server: &str) -> anyhow::Result<()> {
    let server = reqwest::Url::parse(server).context("invalid server URL")?;
    if !matches!(server.scheme(), "http" | "https") {
        bail!("the server URL must use http or https");
    }
    Ok(())
}

impl Target {
    /// A client for this target whose requests carry the token and headers and give up after `timeout`.
    pub fn client(&self, timeout: Duration) -> anyhow::Result<ActorClient> {
        let http = reqwest::Client::builder().default_headers(self.headers()?).timeout(timeout).build()?;
        Ok(ActorClient::with_client(&self.server, http))
    }

    /// The client of a worker over HTTP: the token and headers of [`Self::client`], [`WORKER_TIMEOUT`], and no
    /// redirects, as a `307` or `308` would send a POST of a lease call again. Checks the URL and adds no retry of its
    /// own.
    pub(crate) fn worker_client(&self) -> anyhow::Result<ActorClient> {
        check_server_url(&self.server)?;
        let http = reqwest::Client::builder()
            .default_headers(self.headers()?)
            .timeout(WORKER_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(ActorClient::with_client(&self.server, http))
    }

    /// The headers of every request: those of the context, and the token as a sensitive `Authorization`.
    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            let name = HeaderName::try_from(name.as_str()).with_context(|| format!("invalid header name '{name}'"))?;
            let value = HeaderValue::try_from(value).with_context(|| format!("invalid value for header '{name}'"))?;
            headers.insert(name, value);
        }
        if let Some(token) = &self.token {
            let mut value = HeaderValue::try_from(format!("Bearer {token}")).context("invalid token")?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn config() -> Config {
        toml::from_str(
            r#"
            current-context = "a"
            [contexts.a]
            server = "http://a"
            token = "ta"
            headers = { "X-Tenant" = "ops" }
            [contexts.b]
            server = "http://b"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn the_file_alone_gives_the_current_context() {
        let target = resolve(&Overrides::default(), &Env::new(), &config(), "cicada").unwrap();
        assert_eq!(target.context.as_deref(), Some("a"));
        assert_eq!(target.server, "http://a");
        assert_eq!(target.token.as_deref(), Some("ta"));
        assert_eq!(target.headers["X-Tenant"], "ops");
    }

    #[test]
    fn a_flag_wins_over_the_environment_and_the_environment_over_the_file() {
        let e = env(&[("CICADA_SERVER", "http://env"), ("CICADA_TOKEN", "te")]);
        let target = resolve(&Overrides::default(), &e, &config(), "cicada").unwrap();
        assert_eq!((target.server.as_str(), target.token.as_deref()), ("http://env", Some("te")));

        let flags = Overrides { server: Some("http://flag".into()), token: Some("tf".into()), ..Overrides::default() };
        let target = resolve(&flags, &e, &config(), "cicada").unwrap();
        assert_eq!((target.server.as_str(), target.token.as_deref()), ("http://flag", Some("tf")));
    }

    #[test]
    fn the_context_comes_from_the_flag_else_the_environment() {
        let e = env(&[("CICADA_CONTEXT", "b")]);
        assert_eq!(resolve(&Overrides::default(), &e, &config(), "cicada").unwrap().server, "http://b");
        let flags = Overrides { context: Some("a".into()), ..Overrides::default() };
        assert_eq!(resolve(&flags, &e, &config(), "cicada").unwrap().server, "http://a");
    }

    #[test]
    fn an_unknown_context_is_an_error() {
        let flags = Overrides { context: Some("nope".into()), ..Overrides::default() };
        let error = resolve(&flags, &Env::new(), &config(), "cicada").unwrap_err().to_string();
        assert!(error.contains("'nope'"), "{error}");
    }

    #[test]
    fn no_server_anywhere_says_how_to_set_one() {
        let error = resolve(&Overrides::default(), &Env::new(), &Config::default(), "cicada").unwrap_err().to_string();
        assert!(error.contains("--server") && error.contains("CICADA_SERVER"), "{error}");
    }

    #[test]
    fn a_server_flag_needs_no_config() {
        let flags = Overrides { server: Some("http://x".into()), ..Overrides::default() };
        let target = resolve(&flags, &Env::new(), &Config::default(), "cicada").unwrap();
        assert_eq!(target, Target { context: None, server: "http://x".into(), token: None, headers: BTreeMap::new() });
    }

    #[test]
    fn the_debug_of_a_target_hides_its_credentials() {
        let target = Target {
            context: Some("prod".into()),
            server: "https://user:hunter2@actors.internal/api".into(),
            token: Some("s3cret".into()),
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "k3y".to_owned())]),
        };
        let text = format!("{target:?}");
        for secret in ["hunter2", "s3cret", "k3y"] {
            assert!(!text.contains(secret), "{text}");
        }
        assert!(text.contains("prod") && text.contains("user:***@actors.internal") && text.contains("X-Api-Key"));
    }

    /// A password with a character that ends an address in a text (`'`, `"`, a backtick, `#`, `/`) is hidden whole;
    /// a value that is not a URL keeps only its scheme.
    #[test]
    fn the_debug_of_a_target_hides_any_password_and_an_unparsable_server() {
        let debug = |server: &str| {
            let target = Target { context: None, server: server.to_owned(), token: None, headers: BTreeMap::new() };
            format!("{target:?}")
        };
        for (server, secret) in [
            ("https://user:can'ary@actors.internal/api", "can'ary"),
            ("https://user:a%22b%60c@actors.internal", "a%22b%60c"),
            ("https://user:p%2Fq%23r@actors.internal", "p%2Fq%23r"),
        ] {
            let text = debug(server);
            assert!(!text.contains(secret) && !text.contains("ary"), "{text}");
            assert!(text.contains("https://user:***@actors.internal"), "{text}");
        }
        assert!(debug("http://actors.internal:8080").contains("http://actors.internal:8080"), "no password, as it is");
        for (server, shown) in [("https://user:can'ary@[bad", "https://***"), ("user:can'ary@host", "***")] {
            let text = debug(server);
            assert!(!text.contains("can'ary") && !text.contains("host") && text.contains(shown), "{text}");
        }
    }

    #[test]
    fn the_path_follows_cicada_config_then_xdg_then_home() {
        let all = env(&[("CICADA_CONFIG", "/c.toml"), ("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(path(&all).unwrap(), PathBuf::from("/c.toml"));
        assert_eq!(
            path(&env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")])).unwrap(),
            Path::new("/x/cicada/config.toml")
        );
        assert_eq!(path(&env(&[("HOME", "/h")])).unwrap(), Path::new("/h/.config/cicada/config.toml"));
        assert!(path(&Env::new()).is_err());
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub/config.toml");
        assert_eq!(Config::load(&file).unwrap(), Config::default());
        config().save(&file).unwrap();
        assert_eq!(Config::load(&file).unwrap(), config());
    }

    #[test]
    fn contexts_load_names_and_the_current_context_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let env = env(&[("CICADA_CONFIG", file.to_str().unwrap()), ("CICADA_CONTEXT", "b")]);
        let contexts = Contexts::load(&env).unwrap();
        assert!(contexts.names().is_empty());
        assert_eq!(contexts.current(), None);
        config().save(&file).unwrap();
        let contexts = Contexts::load(&env).unwrap();
        assert_eq!(contexts.names(), ["a", "b"]);
        assert_eq!(contexts.current(), Some("a"));
    }

    #[test]
    fn a_contexts_parse_error_does_not_expose_its_file_contents() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "[contexts.a]\ntoken = invalid-secret").unwrap();
        let env = env(&[("CICADA_CONFIG", file.path().to_str().unwrap())]);
        let error = Contexts::load(&env).unwrap_err();
        assert!(error.to_string().contains("cannot read"));
        assert!(!format!("{error:?}").contains("invalid-secret"));
    }
    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let mode = |file: &Path| std::fs::metadata(file).unwrap().permissions().mode() & 0o777;
        config().save(&file).unwrap();
        assert_eq!(mode(&file), 0o600, "a new file");

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        config().save(&file).unwrap();
        assert_eq!(mode(&file), 0o600, "an existing file with a wider mode");
        assert_eq!(Config::load(&file).unwrap(), config());
    }

    #[cfg(unix)]
    #[test]
    fn a_reader_of_the_old_file_never_reads_the_new_text() {
        use std::io::Read as _;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let old = "[contexts.old]\nserver = \"http://old\"\n";
        std::fs::write(&file, old).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut reader = std::fs::File::open(&file).unwrap();

        config().save(&file).unwrap();
        let mut seen = String::new();
        reader.read_to_string(&mut seen).unwrap();
        assert_eq!(seen, old, "the old descriptor still reads the old file");
        assert_eq!(Config::load(&file).unwrap(), config());
    }

    /// The names in `dir`.
    fn names(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_save_over_a_file_keeps_its_other_contexts_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        config().save(&file).unwrap();
        let mut changed = Config::load(&file).unwrap();
        changed.contexts.insert("c".into(), Context { server: "http://c".into(), ..Context::default() });
        changed.save(&file).unwrap();

        let saved = Config::load(&file).unwrap();
        assert_eq!(saved.contexts.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(saved.contexts["a"], config().contexts["a"], "with its token and headers");
        assert_eq!(names(dir.path()), ["config.toml"]);
    }

    #[test]
    fn a_failed_save_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        std::fs::create_dir(&file).unwrap(); // a directory cannot be replaced by a file
        std::fs::write(file.join("keep"), "").unwrap();
        let error = config().save(&file).unwrap_err().to_string();
        assert!(error.contains("cannot write"), "{error}");
        assert_eq!(names(dir.path()), ["config.toml"]);
    }

    fn file_env(file: &Path) -> Env {
        env(&[("CICADA_CONFIG", file.to_str().unwrap())])
    }

    #[test]
    fn a_context_file_sets_uses_and_saves_and_keeps_headers() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let env = file_env(&file);
        let mut contexts = ContextFile::load(&env).unwrap();
        assert_eq!((contexts.names().count(), contexts.current()), (0, None), "a missing file");
        assert_eq!(contexts.path(), file);
        let headers = BTreeMap::from([("X-Tenant".to_owned(), "ops".to_owned())]);
        contexts.set_context("a", ContextEntry { server: "http://a".into(), token: None, headers: headers.clone() });
        contexts.set_context("b", ContextEntry { server: "http://b".into(), ..ContextEntry::default() });
        assert_eq!(contexts.current(), Some("a"), "the first set context becomes current");
        contexts.save().unwrap();

        let mut contexts = ContextFile::load(&env).unwrap();
        assert_eq!(contexts.names().collect::<Vec<_>>(), ["a", "b"]);
        contexts.set_server_and_token("a", "http://a2".into(), Some("t".into()));
        contexts.set_server_and_token("c", "http://c".into(), None);
        contexts.use_context("b").unwrap();
        let error = contexts.use_context("nope").unwrap_err().to_string();
        assert!(error.contains("no context 'nope' in") && error.contains("config.toml"), "{error}");
        contexts.save().unwrap();

        let contexts = ContextFile::load(&env).unwrap();
        assert_eq!(contexts.current(), Some("b"));
        let a = contexts.context("a").unwrap();
        assert_eq!((a.server.as_str(), a.token.as_deref(), &a.headers), ("http://a2", Some("t"), &headers));
        assert_eq!(contexts.context("c").unwrap(), ContextEntry { server: "http://c".into(), ..Default::default() });
        assert_eq!(contexts.context("nope"), None);
    }

    #[test]
    fn two_updates_from_two_threads_are_both_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let env = file_env(&file);
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for name in ["a", "b"] {
                let (env, barrier) = (&env, &barrier);
                scope.spawn(move || {
                    for i in 0..20 {
                        barrier.wait();
                        ContextFile::update(env, |file| {
                            // Hold the read copy a while, so that an unlocked writer would lose the other's change.
                            std::thread::sleep(Duration::from_millis(1));
                            file.set_server_and_token(&format!("{name}{i}"), format!("http://{name}"), None);
                            Ok(())
                        })
                        .unwrap();
                    }
                });
            }
        });
        assert_eq!(ContextFile::load(&env).unwrap().names().count(), 40);
    }

    #[cfg(unix)]
    #[test]
    fn an_update_through_a_symlink_locks_next_to_its_target_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.toml");
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink("real.toml", &link).unwrap();
        ContextFile::update(&file_env(&link), |file| {
            file.set_context("a", ContextEntry { server: "http://a".into(), ..ContextEntry::default() });
            assert_eq!(file.resolved_path().unwrap(), target);
            Ok(())
        })
        .unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!(dir.path().join("real.toml.lock").exists() && !dir.path().join("config.toml.lock").exists());
        assert_eq!(ContextFile::load(&file_env(&target)).unwrap().current(), Some("a"));
    }

    #[test]
    fn a_failed_update_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        config().save(&file).unwrap();
        let result: anyhow::Result<()> = ContextFile::update(&file_env(&file), |file| {
            file.set_server_and_token("c", "http://c".into(), None);
            bail!("refused")
        });
        assert!(result.is_err());
        assert_eq!(Config::load(&file).unwrap(), config());
    }

    #[test]
    fn a_parse_error_names_the_line_but_not_the_token_for_every_reader() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        std::fs::write(&file, "[contexts.a]\nserver = \"http://a\"\nheaders = \"invalid-secret\"\n").unwrap();
        let env = file_env(&file);
        let errors = [
            Config::load(&file).unwrap_err(),
            ContextFile::load(&env).unwrap_err(),
            ContextFile::update(&env, |_| Ok(())).unwrap_err(),
            Contexts::load(&env).unwrap_err(),
        ];
        for error in errors {
            let text = format!("{error:?} {error:#}");
            assert!(!text.contains("invalid-secret"), "{text}");
            assert!(text.contains("cannot read") && text.contains("line 3"), "{text}");
        }
    }

    #[test]
    fn the_debug_of_a_context_and_the_config_hides_credentials() {
        let entry = ContextEntry {
            server: "https://user:hunter2@actors.internal/api".into(),
            token: Some("s3cret".into()),
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "k3y".to_owned())]),
        };
        let mut file = ContextFile { path: "config.toml".into(), config: Config::default(), changed: false };
        file.set_context("prod", entry.clone());
        for text in [format!("{entry:?}"), format!("{:?}", file.config), format!("{file:?}")] {
            for secret in ["hunter2", "s3cret", "k3y"] {
                assert!(!text.contains(secret), "{text}");
            }
            assert!(text.contains("user:***@actors.internal") && text.contains("X-Api-Key"), "{text}");
        }
    }

    #[test]
    fn a_server_url_must_be_http_or_https() {
        assert!(check_server_url("http://localhost:8080").is_ok());
        assert!(check_server_url("https://actors.internal/api").is_ok());
        let error = format!("{:#}", check_server_url("ftp://x").unwrap_err());
        assert_eq!(error, "the server URL must use http or https");
        let error = format!("{:#}", check_server_url("not a url").unwrap_err());
        assert!(error.starts_with("invalid server URL") && !error.contains("not a url"), "{error}");
    }
}
