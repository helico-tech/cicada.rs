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
use base64::Engine as _;
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
/// readers take no lock. A config path whose name, or whose symlink target's name, is `tray.toml` (the menu bar app's
/// own file) or ends in `.lock` (a lock file), in any letter case, is refused, so that no write replaces either. Its
/// `Debug` hides the credentials as [`ContextEntry`] does.
#[derive(Debug, Clone)]
pub struct ContextFile {
    path: PathBuf,
    config: Config,
    changed: bool,
    /// The file [`Self::update`] locked and reads; its saves write this file, not what the link names by then.
    pinned: Option<PathBuf>,
}

impl ContextFile {
    /// Reads the config file at the path resolved from `env`: `$CICADA_CONFIG`, else
    /// `$XDG_CONFIG_HOME/cicada/config.toml`, else `$HOME/.config/cicada/config.toml`. A missing file has no contexts.
    pub fn load(env: &Env) -> anyhow::Result<Self> {
        let path = path(env)?;
        allowed_target(&path)?;
        let config = Config::load(&path)?;
        Ok(Self { path, config, changed: false, pinned: None })
    }

    /// Locks the config file, reads it again, applies `f` and, when `f` returns `Ok` and changed the file since it
    /// last saved, saves it; then unlocks. Every writer that uses it, `cicada config use-context` and `set-context`
    /// among them, keeps the others' changes. The lock is an exclusive OS lock on `<target>.lock` next to the file a
    /// save writes: the path with every symlink followed, its folders' included, so every path to one file shares one
    /// lock. It is held on this machine only. After locking, the whole path is resolved again: when it names another
    /// file by then, the lock is dropped and taken on that one. The file read and saved is the resolved one locked,
    /// named by a path without symlinks, so a link retargeted later does not move the write. A file system without
    /// locks fails the update rather than risk losing another writer's change. `f` may call [`Self::save`] itself and
    /// keep the lock for more work.
    pub fn update<T>(env: &Env, f: impl FnOnce(&mut Self) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let path = path(env)?;
        let mut tries = 0;
        let (target, _lock) = loop {
            let target = allowed_target(&path)?;
            let lock = lock_next_to(&target)?;
            if real_path(&path)? == target {
                break (target, lock);
            }
            tries += 1;
            if tries == 10 {
                bail!("cannot lock {}: its symlink keeps changing", path.display());
            }
        };
        let config = Config::load(&target)?;
        let mut file = Self { config, path, changed: false, pinned: Some(target) };
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

    /// The file a save writes: [`Self::path`] made absolute with every symlink followed, those of its folders
    /// included. It need not exist.
    pub fn resolved_path(&self) -> std::io::Result<PathBuf> {
        resolve_path(&self.path)
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
        match &self.pinned {
            Some(target) => self.config.write_to(target)?,
            None => self.config.write_to(&allowed_target(&self.path)?)?,
        }
        self.changed = false;
        Ok(())
    }
}

/// [`resolve_path`] with an error that names `path`.
fn real_path(path: &Path) -> anyhow::Result<PathBuf> {
    resolve_path(path).with_context(|| format!("cannot follow {}", path.display()))
}

/// `path` made absolute with every symlink followed, its folders' included: the folder of the file, after the file's
/// own links, is made canonical up to its deepest existing ancestor, and the file's name is checked again in the real
/// folder. The file and some folders need not exist.
fn resolve_path(path: &Path) -> std::io::Result<PathBuf> {
    refuse_folder_form(path)?;
    let mut path = std::path::absolute(path)?;
    for _ in 0..40 {
        let file = link_target(&path)?;
        let name = file.file_name().ok_or_else(|| std::io::Error::other("the path names no file"))?;
        let next = canonical_dir(file.parent().unwrap_or(Path::new("/")))?.join(name);
        if next == path {
            return Ok(next);
        }
        path = next;
    }
    Err(std::io::Error::other("too many levels of symbolic links"))
}

/// Refuses a path that ends in `/` or `/.`: `Path` drops them, but the system refuses such a path for a file
/// (ENOTDIR), so the config code does too, for the path given and for every link target it follows.
fn refuse_folder_form(path: &Path) -> std::io::Result<()> {
    let text = path.as_os_str().as_encoded_bytes();
    if text.ends_with(b"/") || text.ends_with(b"/.") {
        return Err(std::io::Error::other("the path names a folder, not a file"));
    }
    Ok(())
}

/// `dir` made canonical: its deepest existing ancestor through `canonicalize`, with the missing folders after it.
fn canonical_dir(dir: &Path) -> std::io::Result<PathBuf> {
    let mut missing = Vec::new();
    let mut existing = dir;
    loop {
        match std::fs::canonicalize(existing) {
            Ok(real) => return Ok(missing.iter().rev().fold(real, |path, name| path.join(name))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else { return Err(e) };
                missing.push(name);
                existing = parent;
            }
            Err(e) => return Err(e),
        }
    }
}

/// The target of the config path `path`, refused when its name or the target's is reserved: `tray.toml`, the menu bar
/// app's file, or one ending in `.lock`, in any letter case, as a case-insensitive file system treats them alike.
fn allowed_target(path: &Path) -> anyhow::Result<PathBuf> {
    let target = real_path(path)?;
    let reserved = |path: &Path| {
        let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
        name == "tray.toml" || name.ends_with(".lock")
    };
    if reserved(path) || reserved(&target) {
        bail!(
            "the config file {} has a name kept for lock files or tray.toml; set CICADA_CONFIG to another file",
            path.display()
        );
    }
    Ok(target)
}

/// Takes an exclusive lock on `<file>.lock`, creating it and its folder when missing, and waits for another holder.
/// The lock ends when the returned file is dropped. A file system without locks is an error.
pub(crate) fn lock_next_to(file: &Path) -> anyhow::Result<File> {
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
    handle
        .lock()
        .with_context(|| format!("cannot lock {}; its file system must support file locks", lock.display()))?;
    Ok(handle)
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
    /// the diagnostic of the parser can quote a token or server from the file. A path, or a link target on the way,
    /// that ends in `/` or `/.` is refused, also when nothing is there, as the system refuses it for a file; the file
    /// read is the link target that was checked.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        // Read the target that was checked, so a link retargeted after the check cannot change what is read.
        let target = refuse_folder_form(path)
            .and_then(|()| link_target(path))
            .with_context(|| format!("cannot read {}", path.display()))?;
        match std::fs::read_to_string(&target) {
            Ok(text) => toml::from_str(&text).map_err(|error| {
                let before = |at: usize| text.get(..at).map_or(0, |text| text.matches('\n').count());
                let line = error.span().map(|span| before(span.start) + 1);
                let at = line.map(|line| format!(" at line {line}")).unwrap_or_default();
                anyhow!("cannot read {}: not a valid config file{at}", path.display())
            }),
            // Missing only if nothing is at the checked target now either: a link put there after the check, whose
            // target is missing, is refused rather than read as an empty config.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match std::fs::symlink_metadata(&target) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
                Ok(_) => bail!("cannot read {}: it changed while it was read; try again", path.display()),
                Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
            },
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
    #[cfg(test)]
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        self.write_to(&real_path(path)?)
    }

    /// Writes the whole file to `target`, the resolved file of a config path, as [`Self::save`] describes.
    fn write_to(&self, target: &Path) -> anyhow::Result<()> {
        let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let text = toml::to_string_pretty(self)?;
        let write = || -> std::io::Result<()> {
            let mut file = tempfile::Builder::new().prefix(".cicada-config").tempfile_in(dir)?;
            file.write_all(text.as_bytes())?;
            file.as_file().sync_all()?;
            file.persist(target)?;
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
                refuse_folder_form(&link)?;
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

/// The server a command talks to, and how. Its `Debug` leaves out the token and the values of the headers, and hides
/// the password of the URL.
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

/// A client for a context of the config file, resolved as the operator commands resolve their target, with their
/// timeout (30 s), and the name of the context it uses.
///
/// The order is that of `cicada`: `CICADA_SERVER` and `CICADA_TOKEN` win, then the context named by `context`, else
/// by `CICADA_CONTEXT`, else the file's `current-context`. The name returned is that effective context (`None` when no
/// context is used, as with `CICADA_SERVER` alone), so a picker shows what is really used. The file is read as
/// [`ContextFile::load`] reads it.
///
/// No error and no `Debug` of the client holds the credentials of the server URL: the URL is parsed and checked first
/// ([`check_server_url`], whose errors do not quote it), its user and password move into a sensitive
/// `Authorization: Basic` header (which, as before, wins over a token), and the client gets the parsed URL without
/// them.
pub fn context_client(env: &Env, context: Option<&str>) -> anyhow::Result<(ActorClient, Option<String>)> {
    let file = ContextFile::load(env)?;
    let flags = Overrides { context: context.map(str::to_owned), ..Overrides::default() };
    let target = resolve(&flags, env, &file.config, "cicada")?;
    check_server_url(&target.server)?;
    let mut url = reqwest::Url::parse(&target.server).context("invalid server URL")?;
    let mut headers = target.headers()?;
    if !url.username().is_empty() || url.password().is_some() {
        let decode = |text: &str| percent_encoding::percent_decode_str(text).decode_utf8_lossy().into_owned();
        let credentials = format!("{}:{}", decode(url.username()), url.password().map(decode).unwrap_or_default());
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        let mut value = HeaderValue::try_from(format!("Basic {encoded}")).context("invalid server credentials")?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
        // A URL with a user or password has an authority, which takes both.
        let _ = url.set_username("");
        let _ = url.set_password(None);
    }
    let http = reqwest::Client::builder().default_headers(headers).timeout(crate::operator::TIMEOUT).build()?;
    Ok((ActorClient::with_client(url.as_str(), http), target.context))
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

    /// The config file of [`config`] in a new folder, and an environment that names it with `pairs`.
    fn context_env(pairs: &[(&str, &str)]) -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        config().save(&file).unwrap();
        let mut env = env(pairs);
        env.insert("CICADA_CONFIG".into(), file.to_str().unwrap().into());
        (dir, env)
    }

    /// The base URL of a client, read from its `Debug`.
    fn base(client: &ActorClient) -> String {
        let text = format!("{client:?}");
        let start = text.find("base: \"").unwrap() + 7;
        text[start..start + text[start..].find('"').unwrap()].to_owned()
    }

    #[test]
    fn a_context_client_resolves_as_the_operator_commands_and_names_its_context() {
        let (_dir, e) = context_env(&[]);
        let (client, name) = context_client(&e, None).unwrap();
        assert_eq!((base(&client), name.as_deref()), ("http://a".into(), Some("a")), "the file's current context");
        let (client, name) = context_client(&e, Some("b")).unwrap();
        assert_eq!((base(&client), name.as_deref()), ("http://b".into(), Some("b")), "the named context");

        let (_dir, e) = context_env(&[("CICADA_CONTEXT", "b")]);
        let (client, name) = context_client(&e, None).unwrap();
        assert_eq!((base(&client), name.as_deref()), ("http://b".into(), Some("b")), "CICADA_CONTEXT over the file");
        let (_, name) = context_client(&e, Some("a")).unwrap();
        assert_eq!(name.as_deref(), Some("a"), "the named context over CICADA_CONTEXT");

        let (_dir, e) = context_env(&[("CICADA_SERVER", "http://env")]);
        let (client, name) = context_client(&e, Some("b")).unwrap();
        assert_eq!((base(&client), name.as_deref()), ("http://env".into(), Some("b")), "the environment wins");

        let error = context_client(&e, Some("c")).unwrap_err().to_string();
        assert_eq!(error, "no context 'c' in the config file");
    }

    #[test]
    fn a_context_client_without_a_server_says_so_without_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let e = env(&[("CICADA_CONFIG", file.to_str().unwrap()), ("CICADA_TOKEN", "s3cret")]);
        let error = context_client(&e, None).unwrap_err().to_string();
        let operator = resolve(&Overrides::default(), &e, &Config::default(), "cicada").unwrap_err().to_string();
        assert_eq!(error, operator, "the operator commands' text");
        assert!(!error.contains("s3cret"), "{error}");
    }

    /// The client's own parse error quotes its base URL: a server URL that does not parse is refused first, without it.
    #[test]
    fn a_context_client_refuses_a_malformed_server_without_quoting_it() {
        for server in ["http://user:se#cret@host", "http://user:se/cret@host", "localhost:se/cret@8080"] {
            let (_dir, e) = context_env(&[("CICADA_SERVER", server)]);
            let error = format!("{:#}", context_client(&e, None).unwrap_err());
            assert!(error.contains("server URL") && !error.contains("cret") && !error.contains("user"), "{error}");
        }
    }

    /// The credentials of the server URL go into a header; a transport error names the URL without them.
    #[tokio::test]
    async fn a_transport_error_of_a_context_client_hides_the_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (_dir, e) = context_env(&[("CICADA_SERVER", &format!("http://user:s3cret@{address}"))]);
        let (client, _) = context_client(&e, None).unwrap();
        let error = client.health().await.unwrap_err();
        let text = crate::operator::error_text(&anyhow::Error::new(error));
        assert!(text.contains("HTTP request failed") && text.contains(&address.to_string()), "{text}");
        assert!(!text.contains("s3cret") && !text.contains("user"), "{text}");
    }

    /// The review's case: a server URL whose password has a space and that ends in a space passes the URL check (the
    /// parser trims it), but the raw text plus a path does not parse. The client gets the parsed URL without the
    /// credentials, so neither its `Debug` nor an error of a call shows the password.
    #[tokio::test]
    async fn a_malformed_server_url_with_a_password_never_shows_it() {
        let (_dir, e) = context_env(&[("CICADA_SERVER", "http://u:dummy secret@127.0.0.1:81 ")]);
        let (client, _) = context_client(&e, None).unwrap();
        assert_eq!(base(&client), "http://127.0.0.1:81");
        let debug = format!("{client:?}");
        let error = client.health().await.unwrap_err();
        let text = crate::operator::error_text(&anyhow::Error::new(error));
        for shown in [debug, text] {
            assert!(!shown.contains("dummy") && !shown.contains("secret") && !shown.contains("u:"), "{shown}");
        }
    }

    /// The user and password of the URL still reach the server, percent-decoded, as Basic credentials that win over a
    /// token, as reqwest sent them when they were part of the URL.
    #[tokio::test]
    async fn the_credentials_of_a_server_url_are_sent_as_basic_credentials() {
        use axum::http::{HeaderMap, StatusCode};
        let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("us er:p@ss w"));
        let app = axum::Router::new().route(
            "/api/health",
            axum::routing::get(move |headers: HeaderMap| async move {
                let sent = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok());
                if sent == Some(expected.as_str()) { StatusCode::OK } else { StatusCode::UNAUTHORIZED }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let server = format!("http://us%20er:p%40ss%20w@{address}/api");
        let (_dir, e) = context_env(&[("CICADA_SERVER", &server), ("CICADA_TOKEN", "t0ken")]);
        let (client, _) = context_client(&e, None).unwrap();
        assert_eq!(base(&client), format!("http://{address}/api"));
        assert!(client.health().await.unwrap(), "the server saw the Basic credentials");
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
            assert_eq!(file.resolved_path().unwrap(), dir.path().canonicalize().unwrap().join("real.toml"));
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
        let mut file =
            ContextFile { path: "config.toml".into(), config: Config::default(), changed: false, pinned: None };
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

    #[test]
    fn a_config_named_like_a_lock_or_the_tray_file_is_refused_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["tray.toml", "TRAY.TOML", "Tray.Toml", "config.toml.lock", "x.LOCK", "tray.toml.lock"] {
            let file = dir.path().join(name);
            let env = file_env(&file);
            let error = ContextFile::load(&env).unwrap_err().to_string();
            assert!(error.contains("CICADA_CONFIG"), "{name}: {error}");
            let error = ContextFile::update(&env, |file| {
                file.set_server_and_token("a", "http://a".into(), None);
                Ok(())
            })
            .unwrap_err();
            assert!(error.to_string().contains("CICADA_CONFIG"), "{name}");
        }
        assert_eq!(names(dir.path()), Vec::<String>::new(), "nothing created, not even a lock file");
    }

    #[cfg(unix)]
    #[test]
    fn a_config_whose_symlink_names_a_reserved_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink("TRAY.toml", &link).unwrap();
        let error = ContextFile::update(&file_env(&link), |_| Ok(())).unwrap_err().to_string();
        assert!(error.contains("CICADA_CONFIG"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn an_update_saves_the_file_it_locked_when_the_link_moves_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink("first.toml", &link).unwrap();
        ContextFile::update(&file_env(&link), |file| {
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("second.toml", &link).unwrap();
            file.set_server_and_token("a", "http://a".into(), None);
            Ok(())
        })
        .unwrap();
        assert!(Config::load(&dir.path().join("first.toml")).unwrap().contexts.contains_key("a"));
        assert!(!dir.path().join("second.toml").exists(), "not the file whose lock was never taken");
    }

    #[cfg(unix)]
    #[test]
    fn an_update_writes_the_folder_it_locked_when_a_folder_link_moves_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, current) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("current"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::os::unix::fs::symlink("a", &current).unwrap();
        ContextFile::update(&file_env(&current.join("config.toml")), |file| {
            std::fs::remove_file(&current).unwrap();
            std::os::unix::fs::symlink("b", &current).unwrap();
            file.set_server_and_token("x", "http://x".into(), None);
            Ok(())
        })
        .unwrap();
        assert!(Config::load(&a.join("config.toml")).unwrap().contexts.contains_key("x"));
        assert!(!b.join("config.toml").exists(), "not the folder whose lock was never taken");
    }

    /// A writer waiting for the lock of folder `a` while the folder link moves to `b`, whose lock is held, writes `b`
    /// only once it holds `b`'s lock.
    #[cfg(unix)]
    #[test]
    fn a_waiting_update_follows_a_moved_folder_link_and_takes_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, current) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("current"));
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::os::unix::fs::symlink("a", &current).unwrap();
        let held_a = lock_next_to(&a.join("config.toml")).unwrap();
        let env = file_env(&current.join("config.toml"));
        let writer = std::thread::spawn(move || {
            ContextFile::update(&env, |file| {
                file.set_server_and_token("x", "http://x".into(), None);
                Ok(())
            })
        });
        std::thread::sleep(Duration::from_millis(100));
        std::fs::remove_file(&current).unwrap();
        std::os::unix::fs::symlink("b", &current).unwrap();
        let held_b = lock_next_to(&b.join("config.toml")).unwrap();
        drop(held_a);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!b.join("config.toml").exists(), "no write while b's lock is held elsewhere");
        drop(held_b);
        writer.join().unwrap().unwrap();
        assert!(Config::load(&b.join("config.toml")).unwrap().contexts.contains_key("x"));
        assert!(!a.join("config.toml").exists());
    }

    #[test]
    fn a_resolved_path_follows_folder_links_and_keeps_missing_folders() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        #[cfg(unix)]
        {
            std::fs::create_dir(dir.path().join("a")).unwrap();
            std::os::unix::fs::symlink("a", dir.path().join("current")).unwrap();
            let resolved = resolve_path(&dir.path().join("current/config.toml")).unwrap();
            assert_eq!(resolved, real.join("a/config.toml"));
        }
        let resolved = resolve_path(&dir.path().join("new/sub/config.toml")).unwrap();
        assert_eq!(resolved, real.join("new/sub/config.toml"));
    }

    #[test]
    fn a_config_path_with_a_trailing_slash_or_dot_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        let text = "[contexts.a]\nserver = \"http://a\"\n";
        std::fs::write(&file, text).unwrap();
        for suffix in ["/", "/."] {
            let given = PathBuf::from(format!("{}{suffix}", file.display()));
            assert!(resolve_path(&given).is_err(), "{suffix}");
            let env = Env::from([("CICADA_CONFIG".to_owned(), given.display().to_string())]);
            assert!(ContextFile::update(&env, |f| f.use_context("a")).is_err(), "{suffix}");
        }
        #[cfg(unix)]
        for suffix in ["/", "/."] {
            let link = dir.path().join(format!("link{}", suffix.len()));
            std::os::unix::fs::symlink(format!("{}{suffix}", file.display()), &link).unwrap();
            let env = Env::from([("CICADA_CONFIG".to_owned(), link.display().to_string())]);
            assert!(ContextFile::update(&env, |f| f.use_context("a")).is_err(), "link to {suffix}");
            assert!(ContextFile::load(&env).is_err(), "link to {suffix}");
            let dangling = dir.path().join(format!("dangling{}", suffix.len()));
            std::os::unix::fs::symlink(format!("{}{suffix}", dir.path().join("missing.toml").display()), &dangling)
                .unwrap();
            assert!(Config::load(&dangling).is_err(), "dangling link to {suffix}");
            let env = Env::from([("CICADA_CONFIG".to_owned(), dangling.display().to_string())]);
            assert!(Contexts::load(&env).is_err(), "dangling link to {suffix}");
        }
        for suffix in ["/", "/."] {
            let missing = PathBuf::from(format!("{}{suffix}", dir.path().join("missing.toml").display()));
            assert!(Config::load(&missing).is_err(), "missing {suffix}");
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        let env = Env::from([("CICADA_CONFIG".to_owned(), file.display().to_string())]);
        ContextFile::update(&env, |f| f.use_context("a")).unwrap();
    }
}
