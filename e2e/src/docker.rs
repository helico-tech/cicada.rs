//! The docker CLI, the compose project of `e2e/docker-compose.yml`, and single containers started with `docker run`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, bail};

use crate::cluster::{Node, eventually};

/// The image the tests run: `sample-service/Dockerfile`, built once at the start of the suite.
pub const IMAGE: &str = "cicada-rs-sample:e2e";

/// Result of one docker CLI invocation: exit code and output (stdout, then stderr).
#[derive(Debug, Clone)]
pub struct Output {
    /// The exit code; -1 when the process was ended by a signal.
    pub code: i32,
    /// stdout followed by stderr.
    pub text: String,
}

impl Output {
    /// True for exit code 0.
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    fn of(output: &std::process::Output) -> Self {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        Self { code: output.status.code().unwrap_or(-1), text }
    }
}

/// The repository root (the parent of this crate).
pub fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("e2e/ has a parent").to_path_buf())
}

/// The docker CLI: `CICADA_DOCKER`, else `docker` on the `PATH`, else where OrbStack, Docker Desktop or Homebrew put it.
pub fn cli() -> &'static str {
    static CLI: OnceLock<String> = OnceLock::new();
    CLI.get_or_init(|| {
        if let Ok(cli) = std::env::var("CICADA_DOCKER") {
            return cli;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let on_path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).map(|d| d.join("docker")).collect::<Vec<_>>())
            .unwrap_or_default();
        let known = ["/usr/local/bin/docker", "/opt/homebrew/bin/docker", &format!("{home}/.orbstack/bin/docker")]
            .map(PathBuf::from);
        on_path
            .into_iter()
            .chain(known)
            .find(|p| p.is_file())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "docker".to_owned())
    })
}

/// Runs `docker <args>` in the repository root and waits for it without a time limit; for setup and teardown, which
/// also run outside the async runtime. Output is captured.
pub fn docker_blocking(args: &[&str]) -> Output {
    match std::process::Command::new(cli()).args(args).current_dir(root()).stdin(Stdio::null()).output() {
        Ok(output) => Output::of(&output),
        Err(e) => Output { code: -1, text: format!("Cannot run {}: {e}", cli()) },
    }
}

/// Runs `docker <args>` in the repository root, killed after `timeout`. With `check`, a non-zero exit is an error.
pub async fn docker(args: &[&str], timeout: Duration, check: bool) -> anyhow::Result<Output> {
    let command = format!("docker {}", args.join(" "));
    let child = tokio::process::Command::new(cli())
        .args(args)
        .current_dir(root())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(timeout, child)
        .await
        .map_err(|_| anyhow::anyhow!("Timed out after {timeout:?}: {command}"))?
        .with_context(|| format!("Cannot run {command}"))?;
    let output = Output::of(&output);
    if check && !output.ok() {
        bail!("Failed (exit {}): {command}\n{}", output.code, output.text);
    }
    Ok(output)
}

/// Builds [`IMAGE`] from `sample-service/Dockerfile` (docker's layer and cargo caches make a rebuild quick).
///
/// # Panics
///
/// When the build fails.
pub fn build_image() {
    let output = docker_blocking(&["build", "-q", "-f", "sample-service/Dockerfile", "-t", IMAGE, "."]);
    assert!(output.ok(), "docker build failed (exit {}):\n{}", output.code, output.text);
}

/// The `cicada-rs-e2e` compose project of `e2e/docker-compose.yml`.
pub mod compose {
    use super::*;

    /// The compose project name.
    pub const PROJECT: &str = "cicada-rs-e2e";
    /// The PostgreSQL service of the stack.
    pub const POSTGRES: &str = "postgres";

    fn args<'a>(file: &'a str, args: &[&'a str]) -> Vec<&'a str> {
        let mut all = vec!["compose", "-p", PROJECT, "-f", file];
        all.extend_from_slice(args);
        all
    }

    fn file() -> String {
        root().join("e2e/docker-compose.yml").display().to_string()
    }

    /// Runs `docker compose -p cicada-rs-e2e -f e2e/docker-compose.yml <args>`.
    pub async fn run(arguments: &[&str], timeout: Duration, check: bool) -> anyhow::Result<Output> {
        let file = file();
        docker(&args(&file, arguments), timeout, check).await
    }

    /// Starts (or completes) the stack and waits until every service is healthy. Never builds: the suite did.
    pub async fn up() -> anyhow::Result<()> {
        run(&["up", "-d", "--no-build", "--wait", "--wait-timeout", "120"], Duration::from_secs(240), true).await?;
        Ok(())
    }

    /// Removes containers, networks and volumes (blocking; for teardown).
    pub fn down_blocking() -> Output {
        let file = file();
        docker_blocking(&args(&file, &["down", "-v", "--remove-orphans", "--timeout", "5"]))
    }

    /// Everything the containers logged, for post-mortems (blocking; for teardown).
    pub fn logs_blocking() -> String {
        let file = file();
        docker_blocking(&args(&file, &["logs", "--no-color", "--timestamps"])).text
    }

    /// `kill -s SIGKILL`.
    pub async fn kill(service: &str) -> anyhow::Result<()> {
        run(&["kill", "-s", "SIGKILL", service], Duration::from_secs(120), true).await.map(drop)
    }

    /// Freezes every process of the service.
    pub async fn pause(service: &str) -> anyhow::Result<()> {
        run(&["pause", service], Duration::from_secs(120), true).await.map(drop)
    }

    /// Unfreezes the service; not an error when it was not paused.
    pub async fn unpause(service: &str) {
        let _ = run(&["unpause", service], Duration::from_secs(120), false).await;
    }

    /// Graceful stop: SIGTERM, then SIGKILL after the service's `stop_grace_period` (15 s). Returns when the container
    /// has exited.
    pub async fn stop(service: &str) -> anyhow::Result<()> {
        run(&["stop", service], Duration::from_secs(120), true).await.map(drop)
    }

    /// Starts a stopped service again (without waiting for its health check).
    pub async fn start(service: &str, check: bool) -> anyhow::Result<()> {
        run(&["start", service], Duration::from_secs(120), check).await.map(drop)
    }

    /// Restarts every service (graceful stop, then start).
    pub async fn restart() -> anyhow::Result<()> {
        run(&["restart"], Duration::from_secs(240), true).await.map(drop)
    }

    /// The services of running containers whose health check currently reports `unhealthy`.
    pub async fn unhealthy() -> anyhow::Result<Vec<String>> {
        let output = run(&["ps", "--format", "{{.Service}} {{.Health}}"], Duration::from_secs(60), false).await?;
        Ok(output
            .text
            .lines()
            .filter_map(|l| l.trim().split_once(' '))
            .filter(|(_, health)| *health == "unhealthy")
            .map(|(service, _)| service.to_owned())
            .collect())
    }
}

/// A sample-service container started with `docker run`, outside the compose stack.
#[derive(Debug)]
pub struct SingleNode {
    /// The container name, also the node's display name.
    pub name: &'static str,
    port: u16,
    env: Vec<(String, String)>,
    volume: Option<&'static str>,
    /// The node, reachable on the host.
    pub node: Node,
}

impl SingleNode {
    /// A container `name` publishing port 8080 on host port `port`, with `env`, a named `volume` on `/data`, and the
    /// bearer `token` its API requires.
    pub fn new(
        name: &'static str,
        port: u16,
        env: &[(&str, &str)],
        volume: Option<&'static str>,
        token: Option<&str>,
    ) -> Self {
        let env = env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Self { name, port, env, volume, node: Node::new(name, port, true, token) }
    }

    /// Removes an old container of this name and starts a new one.
    pub async fn start(&self) -> anyhow::Result<()> {
        self.remove().await;
        let port = format!("{}:8080", self.port);
        let mut args = vec!["run".to_owned(), "-d".into(), "--name".into(), self.name.into(), "-p".into(), port];
        if let Some(volume) = self.volume {
            args.extend(["-v".to_owned(), format!("{volume}:/data")]);
        }
        for (key, value) in &self.env {
            args.extend(["-e".to_owned(), format!("{key}={value}")]);
        }
        args.push(IMAGE.to_owned());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        docker(&args, Duration::from_secs(120), true).await.map(drop)
    }

    /// Waits until `GET /health` answers 200.
    pub async fn await_healthy(&self) {
        eventually(&format!("{} healthy", self.name), Duration::from_secs(90), async || {
            Ok(self.node.healthy().await.then_some(()))
        })
        .await;
    }

    /// Removes the container and its anonymous volumes (not an error when absent).
    pub async fn remove(&self) {
        let _ = docker(&["rm", "-f", "-v", self.name], Duration::from_secs(60), false).await;
    }

    /// The container's log.
    pub async fn logs(&self) -> String {
        docker(&["logs", "--timestamps", self.name], Duration::from_secs(60), false)
            .await
            .map(|o| o.text)
            .unwrap_or_else(|e| e.to_string())
    }
}
