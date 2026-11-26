//! The config file and which server a command talks to.
//!
//! The file holds named contexts (`[contexts.<name>]`: `server`, optional `token` and `headers`) and the
//! `current-context`. A command's target comes from its flags, else the environment, else the file; [`resolve`] is a
//! pure function of the three, so it is tested without touching the real environment.

use std::collections::BTreeMap;
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
        let Config { contexts, current_context } = Config::load(&path(env)?).map_err(|error| {
            // A TOML parse diagnostic can quote a token or server from the file.
            if error.is::<toml::de::Error>() { anyhow!("{error}") } else { error }
        })?;
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

/// The config file.
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    /// Base URL of the API, e.g. `http://localhost:8080`.
    pub server: String,
    /// Sent as `Authorization: Bearer <token>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Sent with every request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Config {
    /// Reads the file at `path`; a missing file is an empty config.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("cannot read {}", path.display())),
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
        let server = reqwest::Url::parse(&self.server).context("invalid server URL")?;
        if !matches!(server.scheme(), "http" | "https") {
            bail!("the server URL must use http or https");
        }
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
}
