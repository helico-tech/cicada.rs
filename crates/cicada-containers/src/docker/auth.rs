//! The credentials of a pull, found as the Docker CLI finds them: `config.json`, its credential helpers and `auths`.
//!
//! Nothing here logs, and no credential or helper output reaches an error: [`Lookup::source`] only says where the
//! credentials came from, or why there were none.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bollard::auth::DockerCredentials;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::io::AsyncWriteExt;

/// How long a credential helper may take before it is killed and the pull runs without credentials.
pub(super) const HELPER_LIMIT: Duration = Duration::from_secs(10);

/// Docker Hub's key in `config.json` and for a helper, as the CLI writes it.
const HUB_KEY: &str = "https://index.docker.io/v1/";

/// The credentials for one pull, and where they came from.
pub(super) struct Lookup {
    /// What the pull sends; `None` for a pull without credentials.
    pub credentials: Option<DockerCredentials>,
    /// For the message of a failed pull: `credentials from …` or `no credentials …`. Never a secret.
    pub source: String,
}

/// The registry of an image reference.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Registry {
    /// The host as a message names it: `docker.io` for Docker Hub, else as the reference spells it.
    pub host: String,
    /// The key of `config.json` and of a helper: [`HUB_KEY`] for Docker Hub, else the host.
    pub key: String,
}

impl Registry {
    fn is_hub(&self) -> bool {
        self.key == HUB_KEY
    }
}

/// The registry of `image`, as Docker parses it: the first part before a `/` is a registry when it holds a `.` or a
/// `:`, is `localhost` or has an upper-case letter; otherwise, and for Docker Hub's own spellings, Docker Hub.
pub(super) fn registry_of(image: &str) -> Registry {
    let host = match image.split_once('/') {
        Some((first, _))
            if first.contains(['.', ':']) || first == "localhost" || first.chars().any(char::is_uppercase) =>
        {
            first
        }
        _ => "docker.io",
    };
    if is_hub_host(host) {
        Registry { host: "docker.io".into(), key: HUB_KEY.into() }
    } else {
        Registry { host: host.into(), key: host.into() }
    }
}

fn is_hub_host(host: &str) -> bool {
    matches!(host, "docker.io" | "index.docker.io" | "registry-1.docker.io")
}

/// A key of `auths` or `credHelpers` as the CLI compares it: without `http://` or `https://` and from the first `/`.
pub(super) fn normalise(key: &str) -> &str {
    let key = key.strip_prefix("https://").or_else(|| key.strip_prefix("http://")).unwrap_or(key);
    key.split('/').next().unwrap_or(key)
}

/// Whether the key `key` of `config.json` names `registry`; Docker Hub's spellings all name Docker Hub.
fn names(key: &str, registry: &Registry) -> bool {
    let host = normalise(key);
    if registry.is_hub() { is_hub_host(host) } else { host == registry.host }
}

/// The folder of `config.json`: `dir` when set, else `DOCKER_CONFIG`, else `$HOME/.docker`; `None` without either.
pub(super) fn config_dir(dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = dir {
        return Some(dir.to_owned());
    }
    let from_env = |name| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    from_env("DOCKER_CONFIG").or_else(|| from_env("HOME").map(|home| home.join(".docker")))
}

/// The parts of `config.json` a pull needs.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Config {
    #[serde(default)]
    auths: Map<String, Value>,
    #[serde(default, rename = "credHelpers")]
    cred_helpers: Map<String, Value>,
    #[serde(default, rename = "credsStore")]
    creds_store: Option<String>,
}

impl Config {
    /// `config.json` in `dir`; `None` when it is missing, unreadable or not such a JSON object.
    pub(super) async fn read(dir: &Path) -> Option<Self> {
        let bytes = tokio::fs::read(dir.join("config.json")).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The helper for `registry`: the one `credHelpers` names for it, else `credsStore`.
    fn helper(&self, registry: &Registry) -> Option<&str> {
        let named = self.cred_helpers.iter().find(|(key, _)| names(key, registry)).and_then(|(_, name)| name.as_str());
        named.or(self.creds_store.as_deref()).filter(|name| !name.is_empty())
    }

    /// The credentials of the `auths` entry for `registry`: its exact key first, then a normalised one, as the CLI
    /// does. `None` for an entry with neither a valid `auth` nor an `identitytoken`.
    fn entry(&self, registry: &Registry) -> Option<DockerCredentials> {
        let entry = self
            .auths
            .get(&registry.key)
            .or_else(|| self.auths.iter().find(|(key, _)| names(key, registry)).map(|(_, entry)| entry))?;
        let text = |field| entry.get(field).and_then(Value::as_str).filter(|value| !value.is_empty());
        let basic = text("auth")
            .and_then(|auth| STANDARD.decode(auth).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|pair| pair.split_once(':').map(|(user, password)| (user.to_owned(), password.to_owned())));
        let token = text("identitytoken").map(str::to_owned);
        if basic.is_none() && token.is_none() {
            return None;
        }
        let (username, password) = basic.unzip();
        Some(DockerCredentials {
            username,
            password,
            identitytoken: token,
            serveraddress: Some(registry.key.clone()),
            ..Default::default()
        })
    }

    /// The credentials for `registry` by the CLI's order: the helper when one is configured (its `auths` entry is then
    /// not used), else the `auths` entry. A helper is run as `docker-credential-<name>` within `limit`, from `PATH`,
    /// or from `helpers` when set (for tests, which cannot change `PATH`).
    pub(super) async fn credentials(&self, registry: &Registry, limit: Duration, helpers: Option<&Path>) -> Lookup {
        if let Some(name) = self.helper(registry) {
            let program = format!("docker-credential-{name}");
            let path = match helpers {
                Some(dir) => dir.join(&program).into_os_string(),
                None => program.clone().into(),
            };
            return match run_helper(&path, &registry.key, limit).await {
                Ok(credentials) => {
                    Lookup { credentials: Some(credentials), source: format!("credentials from {program}") }
                }
                Err(reason) => Lookup { credentials: None, source: format!("no credentials: {program} {reason}") },
            };
        }
        match self.entry(registry) {
            Some(credentials) => {
                Lookup { credentials: Some(credentials), source: "credentials from config.json".into() }
            }
            None => no_credentials(registry),
        }
    }
}

fn no_credentials(registry: &Registry) -> Lookup {
    Lookup { credentials: None, source: format!("no credentials for {}", registry.host) }
}

/// The credentials for a pull of `image`, read now from `config.json` in [`config_dir`] of `dir`. A missing or broken
/// file, a registry without an entry and a failing helper give a pull without credentials.
pub(super) async fn lookup(dir: Option<&Path>, image: &str, limit: Duration) -> Lookup {
    let registry = registry_of(image);
    let config = match config_dir(dir) {
        Some(dir) => Config::read(&dir).await,
        None => None,
    };
    match config {
        Some(config) => config.credentials(&registry, limit, None).await,
        None => no_credentials(&registry),
    }
}

/// The answer of a helper's `get`.
#[derive(Deserialize)]
struct HelperAnswer {
    #[serde(rename = "Username")]
    username: String,
    #[serde(rename = "Secret")]
    secret: String,
}

/// Runs `<program> get` with `server` on stdin and reads its JSON answer; a `Username` of `<token>` makes the secret
/// an identity token. Killed after `limit`. The error is only a reason: `not found`, `exited with <code>`,
/// `timed out` or `gave no JSON`; the helper's output, stderr included, never leaves this function.
pub(super) async fn run_helper(program: &OsStr, server: &str, limit: Duration) -> Result<DockerCredentials, String> {
    let mut child = tokio::process::Command::new(program)
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "not found".to_owned())?;
    let mut stdin = child.stdin.take();
    let run = async move {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(server.as_bytes()).await; // a helper that exits without reading is judged below
        }
        drop(stdin);
        child.wait_with_output().await
    };
    // A timeout drops `run` with the child, which `kill_on_drop` kills.
    let output = match tokio::time::timeout(limit, run).await {
        Err(_) => return Err("timed out".into()),
        Ok(Err(_)) => return Err("gave no JSON".into()),
        Ok(Ok(output)) => output,
    };
    if !output.status.success() {
        return Err(match output.status.code() {
            Some(code) => format!("exited with {code}"),
            None => "exited with a signal".into(),
        });
    }
    let answer: HelperAnswer = serde_json::from_slice(&output.stdout).map_err(|_| "gave no JSON".to_owned())?;
    let server = Some(server.to_owned());
    Ok(if answer.username == "<token>" {
        DockerCredentials { identitytoken: Some(answer.secret), serveraddress: server, ..Default::default() }
    } else {
        DockerCredentials {
            username: Some(answer.username),
            password: Some(answer.secret),
            serveraddress: server,
            ..Default::default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> Config {
        serde_json::from_str(json).unwrap()
    }

    fn basic(user: &str, password: &str) -> String {
        STANDARD.encode(format!("{user}:{password}"))
    }

    #[test]
    fn the_registry_of_an_image_is_found_as_docker_finds_it() {
        let hub = Registry { host: "docker.io".into(), key: HUB_KEY.into() };
        for image in ["alpine", "alpine:3", "library/alpine", "docker.io/x", "index.docker.io/library/x", "team/app:1"]
        {
            assert_eq!(registry_of(image), hub, "{image}");
        }
        let at = |host: &str| Registry { host: host.into(), key: host.into() };
        assert_eq!(registry_of("ghcr.io/o/x"), at("ghcr.io"));
        assert_eq!(registry_of("localhost:5000/x"), at("localhost:5000"));
        assert_eq!(registry_of("localhost/x"), at("localhost"));
        assert_eq!(registry_of("host:5000/x@sha256:0123abcd"), at("host:5000"));
        assert_eq!(registry_of("registry.gitlab.com/group/project/image:tag"), at("registry.gitlab.com"));
    }

    #[test]
    fn keys_are_normalised_as_the_cli_does() {
        assert_eq!(normalise("https://ghcr.io"), "ghcr.io");
        assert_eq!(normalise("http://localhost:5000/v2/"), "localhost:5000");
        assert_eq!(normalise("ghcr.io"), "ghcr.io");
        assert_eq!(normalise(HUB_KEY), "index.docker.io");
        let hub = registry_of("alpine");
        for key in [HUB_KEY, "docker.io", "index.docker.io", "https://registry-1.docker.io/v2/"] {
            assert!(names(key, &hub), "{key}");
        }
        assert!(names("https://ghcr.io", &registry_of("ghcr.io/o/x")));
        assert!(!names("ghcr.io", &hub));
        assert!(!names("docker.io", &registry_of("ghcr.io/o/x")));
    }

    #[test]
    fn an_auths_entry_gives_basic_credentials_and_an_identity_token() {
        let json = format!(
            r#"{{"auths": {{"https://ghcr.io": {{"auth": "{}"}}, "registry.gitlab.com": {{"identitytoken": "tok"}},
               "localhost:5000": {{}}, "bad.example": {{"auth": "%%%"}}}}}}"#,
            basic("user", "pa:ss:word")
        );
        let config = config(&json);
        let ghcr = config.entry(&registry_of("ghcr.io/o/x")).unwrap();
        assert_eq!(ghcr.username.as_deref(), Some("user"));
        assert_eq!(ghcr.password.as_deref(), Some("pa:ss:word"), "split at the first `:`");
        assert_eq!(ghcr.serveraddress.as_deref(), Some("ghcr.io"));
        let gitlab = config.entry(&registry_of("registry.gitlab.com/g/p")).unwrap();
        assert_eq!((gitlab.username, gitlab.identitytoken.as_deref()), (None, Some("tok")));
        assert!(config.entry(&registry_of("localhost:5000/x")).is_none(), "an empty entry");
        assert!(config.entry(&registry_of("bad.example/x")).is_none(), "not base64");
        assert!(config.entry(&registry_of("alpine")).is_none(), "no entry");
    }

    #[test]
    fn the_exact_key_wins_over_a_normalised_one() {
        let json = format!(
            r#"{{"auths": {{"https://index.docker.io/v1/x": {{"auth": "{}"}}, "{HUB_KEY}": {{"auth": "{}"}}}}}}"#,
            basic("other", "o"),
            basic("hub", "h")
        );
        assert_eq!(config(&json).entry(&registry_of("alpine")).unwrap().username.as_deref(), Some("hub"));
    }

    #[test]
    fn a_helper_is_chosen_by_the_cli_order() {
        let both = config(r#"{"credsStore": "desktop", "credHelpers": {"https://ghcr.io": "gh", "docker.io": "hub"}}"#);
        assert_eq!(both.helper(&registry_of("ghcr.io/o/x")), Some("gh"));
        assert_eq!(both.helper(&registry_of("alpine")), Some("hub"));
        assert_eq!(both.helper(&registry_of("quay.io/o/x")), Some("desktop"));
        let none = config(r#"{"credsStore": "", "auths": {}}"#);
        assert_eq!(none.helper(&registry_of("alpine")), None);
    }

    #[tokio::test]
    async fn a_configured_helper_means_auths_is_not_used() {
        let json = format!(
            r#"{{"credsStore": "cicada-no-such-helper", "auths": {{"ghcr.io": {{"auth": "{}"}}}}}}"#,
            basic("u", "p")
        );
        let lookup = config(&json).credentials(&registry_of("ghcr.io/o/x"), HELPER_LIMIT, None).await;
        assert!(lookup.credentials.is_none());
        assert_eq!(lookup.source, "no credentials: docker-credential-cicada-no-such-helper not found");
    }

    #[tokio::test]
    async fn a_missing_or_broken_file_and_a_missing_entry_give_no_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let missing = lookup(Some(dir.path()), "ghcr.io/o/x", HELPER_LIMIT).await;
        assert!(missing.credentials.is_none());
        assert_eq!(missing.source, "no credentials for ghcr.io");

        std::fs::write(dir.path().join("config.json"), "{not json").unwrap();
        assert_eq!(lookup(Some(dir.path()), "ghcr.io/o/x", HELPER_LIMIT).await.source, "no credentials for ghcr.io");
        std::fs::write(dir.path().join("config.json"), r#"{"auths": []}"#).unwrap();
        assert_eq!(lookup(Some(dir.path()), "alpine", HELPER_LIMIT).await.source, "no credentials for docker.io");

        let json = format!(r#"{{"auths": {{"https://ghcr.io": {{"auth": "{}"}}}}}}"#, basic("u", "secret-1"));
        std::fs::write(dir.path().join("config.json"), json).unwrap();
        let found = lookup(Some(dir.path()), "ghcr.io/o/x:1", HELPER_LIMIT).await;
        assert_eq!(found.source, "credentials from config.json");
        assert_eq!(found.credentials.unwrap().password.as_deref(), Some("secret-1"));
        assert_eq!(lookup(Some(dir.path()), "quay.io/o/x", HELPER_LIMIT).await.source, "no credentials for quay.io");
    }

    #[test]
    fn the_folder_set_on_the_runtime_comes_first() {
        assert_eq!(config_dir(Some(Path::new("/srv/x/.docker"))), Some(PathBuf::from("/srv/x/.docker")));
    }

    #[cfg(unix)]
    mod helpers {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        /// A helper script `docker-credential-<name>` in `dir` that runs `body` in `sh`.
        fn helper(dir: &Path, name: &str, body: &str) -> PathBuf {
            let path = dir.join(format!("docker-credential-{name}"));
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        const LIMIT: Duration = Duration::from_secs(5);

        #[tokio::test]
        async fn a_helper_answers_with_the_server_it_was_given() {
            let dir = tempfile::tempdir().unwrap();
            // Echoes the server key from stdin as the user name.
            let path = helper(
                dir.path(),
                "echo",
                r#"read s; printf '{"ServerURL":"%s","Username":"%s","Secret":"s3"}' "$s" "$s""#,
            );
            let credentials = run_helper(path.as_os_str(), "ghcr.io", LIMIT).await.unwrap();
            assert_eq!(credentials.username.as_deref(), Some("ghcr.io"));
            assert_eq!(credentials.password.as_deref(), Some("s3"));
            assert_eq!((credentials.serveraddress.as_deref(), credentials.identitytoken), (Some("ghcr.io"), None));
        }

        #[tokio::test]
        async fn a_token_user_makes_an_identity_token() {
            let dir = tempfile::tempdir().unwrap();
            let path = helper(dir.path(), "token", r#"cat >/dev/null; echo '{"Username":"<token>","Secret":"id-1"}'"#);
            let credentials = run_helper(path.as_os_str(), HUB_KEY, LIMIT).await.unwrap();
            assert_eq!((credentials.username, credentials.password), (None, None));
            assert_eq!(credentials.identitytoken.as_deref(), Some("id-1"));
            assert_eq!(credentials.serveraddress.as_deref(), Some(HUB_KEY));
        }

        #[tokio::test]
        async fn a_failing_helper_gives_only_a_reason() {
            let dir = tempfile::tempdir().unwrap();
            let failing = helper(dir.path(), "fail", "echo 'credentials not found: secret-out' >&2; exit 3");
            assert_eq!(run_helper(failing.as_os_str(), "ghcr.io", LIMIT).await.unwrap_err(), "exited with 3");
            let chatty = helper(dir.path(), "chatty", "cat >/dev/null; echo 'secret-out is not JSON'");
            assert_eq!(run_helper(chatty.as_os_str(), "ghcr.io", LIMIT).await.unwrap_err(), "gave no JSON");
            let missing = dir.path().join("docker-credential-missing");
            assert_eq!(run_helper(missing.as_os_str(), "ghcr.io", LIMIT).await.unwrap_err(), "not found");
        }

        /// Scenario: Credentials from a helper. The `credsStore` helper answers, and the `auths` entry is not used.
        #[tokio::test]
        async fn a_configured_helper_gives_the_credentials_instead_of_auths() {
            let dir = tempfile::tempdir().unwrap();
            helper(dir.path(), "cicada-test", r#"read s; printf '{"Username":"from-helper","Secret":"for-%s"}' "$s""#);
            let json = format!(
                r#"{{"credsStore": "cicada-test", "auths": {{"ghcr.io": {{"auth": "{}"}}}}}}"#,
                basic("u", "p")
            );
            let lookup = config(&json).credentials(&registry_of("ghcr.io/o/x"), LIMIT, Some(dir.path())).await;
            assert_eq!(lookup.source, "credentials from docker-credential-cicada-test");
            let credentials = lookup.credentials.unwrap();
            assert_eq!(credentials.username.as_deref(), Some("from-helper"));
            assert_eq!(credentials.password.as_deref(), Some("for-ghcr.io"));
            assert_eq!(credentials.serveraddress.as_deref(), Some("ghcr.io"));
        }

        /// Real time: a paused clock would fire the limit at once while the child runs.
        #[tokio::test]
        async fn a_helper_over_its_limit_is_killed() {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("still-running");
            let body = format!("sleep 2; touch {}", marker.display());
            let slow = helper(dir.path(), "slow", &body);
            let started = std::time::Instant::now();
            let reason = run_helper(slow.as_os_str(), "ghcr.io", Duration::from_millis(200)).await.unwrap_err();
            assert_eq!(reason, "timed out");
            assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
            tokio::time::sleep(Duration::from_millis(2500)).await;
            assert!(!marker.exists(), "the helper was killed");
        }
    }
}
