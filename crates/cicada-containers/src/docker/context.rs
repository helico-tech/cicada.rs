//! Which daemon [`DockerRuntime::connect`](super::DockerRuntime::connect) uses: the order of `DOCKER_HOST`,
//! `DOCKER_CONTEXT`, the local socket and the `currentContext` of the Docker CLI's `config.json`.
//!
//! [`resolve`] is a function of its [`Inputs`], which `connect` gathers from the process, so the tests set no
//! environment variable. It reads only the files of the config folder it is given.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{ContainerError, ErrorKind};

/// The local socket that `connect` uses when it exists and no context is named.
pub(super) const LOCAL_SOCKET: &str = "/var/run/docker.sock";

/// The context that stands for the local socket or `DOCKER_HOST`, as in the CLI.
const DEFAULT_CONTEXT: &str = "default";

/// What `connect` reads from the process, as values.
pub(super) struct Inputs<'a> {
    /// Whether `DOCKER_HOST` is set.
    pub docker_host: bool,
    /// `DOCKER_CONTEXT`; an empty one counts as unset.
    pub docker_context: Option<&'a str>,
    /// The CLI's config folder: `DOCKER_CONFIG`, else `$HOME/.docker`; `None` when neither is set.
    pub config_dir: Option<&'a Path>,
    /// Whether [`LOCAL_SOCKET`] exists.
    pub socket_exists: bool,
}

/// The daemon to connect to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Endpoint {
    /// `DOCKER_HOST`, else the local socket, as bollard's defaults read them.
    Defaults,
    /// The docker endpoint of a CLI context.
    Context(Context),
}

/// A CLI context and its docker endpoint.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Context {
    /// The context's name.
    pub name: String,
    /// Where the name came from.
    pub source: Source,
    /// The endpoint, `unix://`, `tcp://` or `http://`.
    pub host: String,
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "context {} from {}", self.name, self.source)
    }
}

/// Where the name of a context came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    /// The variable `DOCKER_CONTEXT`.
    Env,
    /// The `currentContext` of `config.json`.
    Config,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Env => "DOCKER_CONTEXT",
            Self::Config => "currentContext",
        })
    }
}

/// The endpoint for `inputs`: `DOCKER_HOST`; else the context `DOCKER_CONTEXT` names (`default`: the local socket),
/// even when the local socket exists; else the local socket when it exists; else the `currentContext` of
/// `config.json`. A missing or unreadable `config.json`, one without `currentContext`, or the context `default` gives
/// [`Endpoint::Defaults`]. A named context that cannot be read or used fails with `Other`, naming it and its source.
pub(super) fn resolve(inputs: &Inputs<'_>) -> Result<Endpoint, ContainerError> {
    if inputs.docker_host {
        return Ok(Endpoint::Defaults);
    }
    if let Some(name) = inputs.docker_context.filter(|name| !name.is_empty()) {
        if name == DEFAULT_CONTEXT {
            return Ok(Endpoint::Defaults);
        }
        let Some(dir) = inputs.config_dir else {
            return Err(refused(name, Source::Env, "has no config folder: neither DOCKER_CONFIG nor HOME is set"));
        };
        return read(dir, name, Source::Env).map(Endpoint::Context);
    }
    if inputs.socket_exists {
        return Ok(Endpoint::Defaults);
    }
    let Some(dir) = inputs.config_dir else { return Ok(Endpoint::Defaults) };
    match current_context(dir) {
        Some(name) if name != DEFAULT_CONTEXT => read(dir, &name, Source::Config).map(Endpoint::Context),
        _ => Ok(Endpoint::Defaults),
    }
}

/// The non-empty `currentContext` of `config.json` in `dir`; `None` when the file is missing, unreadable or has none.
fn current_context(dir: &Path) -> Option<String> {
    let bytes = std::fs::read(dir.join("config.json")).ok()?;
    let config: Value = serde_json::from_slice(&bytes).ok()?;
    config.get("currentContext")?.as_str().filter(|name| !name.is_empty()).map(str::to_owned)
}

/// The folder name of a context under `contexts/meta` and `contexts/tls`: the lower-case hex SHA-256 of its name.
fn digest(name: &str) -> String {
    Sha256::digest(name.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The meta file of the context `name` in the config folder `dir`.
fn meta_path(dir: &Path, name: &str) -> PathBuf {
    dir.join("contexts").join("meta").join(digest(name)).join("meta.json")
}

/// The docker endpoint of the context `name`, as the CLI stores it: `Endpoints.docker.Host` of its meta file. TLS
/// and schemes other than `unix`, `tcp` and `http` are refused.
fn read(dir: &Path, name: &str, source: Source) -> Result<Context, ContainerError> {
    let path = meta_path(dir, name);
    let bytes = std::fs::read(&path)
        .map_err(|e| refused(name, source, &format!("has no meta file {}: {e}", path.display())))?;
    let meta: Value = serde_json::from_slice(&bytes)
        .map_err(|e| refused(name, source, &format!("has a broken meta file {}: {e}", path.display())))?;
    let docker = meta.get("Endpoints").and_then(|endpoints| endpoints.get("docker"));
    let Some(host) = docker.and_then(|docker| docker.get("Host")).and_then(Value::as_str).filter(|h| !h.is_empty())
    else {
        return Err(refused(name, source, "has no docker endpoint"));
    };
    let skip_verify = docker.and_then(|docker| docker.get("SkipTLSVerify")).and_then(Value::as_bool) == Some(true);
    if skip_verify || dir.join("contexts").join("tls").join(digest(name)).join("docker").exists() {
        return Err(refused(
            name,
            source,
            "uses TLS, which connect() does not support; set DOCKER_HOST or use connect_to",
        ));
    }
    if !["unix://", "tcp://", "http://"].iter().any(|scheme| host.starts_with(scheme)) {
        return Err(refused(
            name,
            source,
            &format!(
                "has the endpoint {host}, whose scheme connect() does not support; set DOCKER_HOST or use connect_to"
            ),
        ));
    }
    Ok(Context { name: name.to_owned(), source, host: host.to_owned() })
}

/// The `Other` error for the context `name` from `source`: `connect to docker: context <name> from <source> <why>`.
fn refused(name: &str, source: Source, why: &str) -> ContainerError {
    ContainerError::new(ErrorKind::Other, format!("connect to docker: context {name} from {source} {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config folder with `config.json` holding `current`, if any, and no contexts yet.
    fn folder(current: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if let Some(current) = current {
            std::fs::write(dir.path().join("config.json"), format!(r#"{{"currentContext": "{current}"}}"#)).unwrap();
        }
        dir
    }

    /// Writes `meta` as the meta file of the context `name` in `dir`.
    fn context(dir: &Path, name: &str, meta: &str) {
        let path = meta_path(dir, name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, meta).unwrap();
    }

    /// A meta file whose docker endpoint is `host`.
    fn meta(host: &str) -> String {
        format!(r#"{{"Name": "x", "Endpoints": {{"docker": {{"Host": "{host}", "SkipTLSVerify": false}}}}}}"#)
    }

    fn inputs<'a>(
        docker_host: bool,
        docker_context: Option<&'a str>,
        config_dir: Option<&'a Path>,
        socket_exists: bool,
    ) -> Inputs<'a> {
        Inputs { docker_host, docker_context, config_dir, socket_exists }
    }

    fn found(name: &str, source: Source, host: &str) -> Endpoint {
        Endpoint::Context(Context { name: name.into(), source, host: host.into() })
    }

    fn other(inputs: &Inputs<'_>) -> String {
        let error = resolve(inputs).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Other, "{error}");
        error.message
    }

    const ORB: &str = "unix:///Users/me/.orbstack/run/docker.sock";

    #[test]
    fn the_meta_folder_is_the_sha256_of_the_name() {
        let path = meta_path(Path::new("/home/me/.docker"), "orbstack");
        assert_eq!(
            path,
            Path::new(
                "/home/me/.docker/contexts/meta/2d89b732b01a00a2d1675ed3cee9fd0f965daadf90603c989dd3afd4569c6896/meta.json"
            )
        );
    }

    /// Scenario: A Mac without the default socket.
    #[test]
    fn without_a_socket_the_current_context_is_used() {
        let dir = folder(Some("orbstack"));
        context(dir.path(), "orbstack", &meta(ORB));
        let endpoint = resolve(&inputs(false, None, Some(dir.path()), false)).unwrap();
        assert_eq!(endpoint, found("orbstack", Source::Config, ORB));
    }

    #[test]
    fn docker_host_wins_over_everything() {
        let dir = folder(Some("orbstack"));
        context(dir.path(), "orbstack", &meta(ORB));
        for socket in [false, true] {
            let endpoint = resolve(&inputs(true, Some("orbstack"), Some(dir.path()), socket)).unwrap();
            assert_eq!(endpoint, Endpoint::Defaults);
        }
    }

    /// Scenario: A context set in the environment.
    #[test]
    fn docker_context_wins_over_the_socket_and_the_current_context() {
        let dir = folder(Some("other"));
        context(dir.path(), "colima", &meta("tcp://127.0.0.1:2375"));
        for socket in [false, true] {
            let endpoint = resolve(&inputs(false, Some("colima"), Some(dir.path()), socket)).unwrap();
            assert_eq!(endpoint, found("colima", Source::Env, "tcp://127.0.0.1:2375"));
        }
    }

    #[test]
    fn docker_context_default_is_the_local_socket() {
        let dir = folder(Some("orbstack"));
        context(dir.path(), "orbstack", &meta(ORB));
        for socket in [false, true] {
            let endpoint = resolve(&inputs(false, Some("default"), Some(dir.path()), socket)).unwrap();
            assert_eq!(endpoint, Endpoint::Defaults, "socket exists: {socket}");
        }
    }

    #[test]
    fn an_empty_docker_context_counts_as_unset() {
        let dir = folder(Some("orbstack"));
        context(dir.path(), "orbstack", &meta(ORB));
        assert_eq!(resolve(&inputs(false, Some(""), Some(dir.path()), true)).unwrap(), Endpoint::Defaults);
        let endpoint = resolve(&inputs(false, Some(""), Some(dir.path()), false)).unwrap();
        assert_eq!(endpoint, found("orbstack", Source::Config, ORB));
    }

    /// Scenario: A server keeps its socket.
    #[test]
    fn the_current_context_is_ignored_while_the_socket_exists() {
        let dir = folder(Some("orbstack"));
        context(dir.path(), "orbstack", &meta(ORB));
        assert_eq!(resolve(&inputs(false, None, Some(dir.path()), true)).unwrap(), Endpoint::Defaults);
        // Even a context that could not be used.
        let dir = folder(Some("missing"));
        assert_eq!(resolve(&inputs(false, None, Some(dir.path()), true)).unwrap(), Endpoint::Defaults);
    }

    #[test]
    fn no_config_json_no_current_context_or_default_keeps_todays_error() {
        let missing = folder(None);
        let empty = folder(None);
        std::fs::write(empty.path().join("config.json"), r#"{"auths": {}}"#).unwrap();
        let broken = folder(None);
        std::fs::write(broken.path().join("config.json"), "{not json").unwrap();
        let default = folder(Some("default"));
        for dir in [&missing, &empty, &broken, &default] {
            assert_eq!(resolve(&inputs(false, None, Some(dir.path()), false)).unwrap(), Endpoint::Defaults);
        }
        assert_eq!(resolve(&inputs(false, None, None, false)).unwrap(), Endpoint::Defaults);
    }

    /// Scenario: A context that does not exist; A context from config.json that does not exist.
    #[test]
    fn a_missing_context_names_it_and_its_source() {
        let dir = folder(Some("gone"));
        let message = other(&inputs(false, Some("absent"), Some(dir.path()), true));
        assert!(
            message.starts_with("connect to docker: context absent from DOCKER_CONTEXT has no meta file "),
            "{message}"
        );
        let message = other(&inputs(false, None, Some(dir.path()), false));
        assert!(
            message.starts_with("connect to docker: context gone from currentContext has no meta file "),
            "{message}"
        );
    }

    #[test]
    fn a_context_without_a_docker_endpoint_or_with_broken_json_is_refused() {
        let dir = folder(None);
        context(dir.path(), "k8s", r#"{"Name": "k8s", "Endpoints": {"kubernetes": {"Host": "https://k8s"}}}"#);
        context(dir.path(), "blank", r#"{"Name": "blank", "Endpoints": {"docker": {"Host": ""}}}"#);
        context(dir.path(), "broken", "{\"Name\": ");
        for (name, why) in [("k8s", "has no docker endpoint"), ("blank", "has no docker endpoint")] {
            let message = other(&inputs(false, Some(name), Some(dir.path()), false));
            assert_eq!(message, format!("connect to docker: context {name} from DOCKER_CONTEXT {why}"));
        }
        let message = other(&inputs(false, Some("broken"), Some(dir.path()), false));
        assert!(message.starts_with("connect to docker: context broken from DOCKER_CONTEXT has a broken meta file "));
    }

    /// Scenario: No config folder.
    #[test]
    fn docker_context_without_a_config_folder_is_refused() {
        let message = other(&inputs(false, Some("orbstack"), None, true));
        assert_eq!(
            message,
            "connect to docker: context orbstack from DOCKER_CONTEXT has no config folder: neither DOCKER_CONFIG nor \
             HOME is set"
        );
    }

    /// Scenario: A TLS context.
    #[test]
    fn tls_contexts_and_other_schemes_are_refused_without_a_fallback() {
        let dir = folder(Some("tls-files"));
        context(
            dir.path(),
            "skip",
            r#"{"Name": "skip", "Endpoints": {"docker": {"Host": "tcp://docker:2376", "SkipTLSVerify": true}}}"#,
        );
        context(dir.path(), "tls-files", &meta("tcp://docker:2376"));
        std::fs::create_dir_all(dir.path().join("contexts/tls").join(digest("tls-files")).join("docker")).unwrap();
        let tls = "uses TLS, which connect() does not support; set DOCKER_HOST or use connect_to";
        let message = other(&inputs(false, Some("skip"), Some(dir.path()), true));
        assert_eq!(message, format!("connect to docker: context skip from DOCKER_CONTEXT {tls}"));
        let message = other(&inputs(false, None, Some(dir.path()), false));
        assert_eq!(message, format!("connect to docker: context tls-files from currentContext {tls}"));

        for host in ["ssh://me@docker", "npipe:////./pipe/docker_engine", "https://docker:2376"] {
            context(dir.path(), "remote", &meta(host));
            let message = other(&inputs(false, Some("remote"), Some(dir.path()), true));
            assert_eq!(
                message,
                format!(
                    "connect to docker: context remote from DOCKER_CONTEXT has the endpoint {host}, whose scheme \
                     connect() does not support; set DOCKER_HOST or use connect_to"
                )
            );
        }
    }
}
