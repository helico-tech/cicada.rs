//! The settings of `serve` and `worker`: the store, the address, the types and the node. Each comes from its flag, else
//! its variable `CICADA_…` (a blank one counts as unset), else for the node the host's `NodeConfig`, else the default.
//! `CICADA_NODE_LABEL` alone is read as given: an empty one counts as unset and a blank one is an error.
//! Parsing the flags and variables is separate from applying them to the host's config.
//! A worker first picks its mode ([`worker_mode`]): on the store it resolves as above, over HTTP ([`parse_http`]) it
//! takes only the settings a [`WorkerConfig`] has and refuses those the server owns (ADR 0042).

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use cicada::runtime::{NodeConfig, WorkerConfig};
use cicada::store::NodeId;
use clap::{Args, Parser};

use crate::Env;
use crate::config::{self, Config, Overrides, Target};

/// Where `serve` listens unless told otherwise: the loopback address, as `App` has no authentication of its own.
pub(crate) const DEFAULT_LISTEN: &str = "127.0.0.1:8080";

/// The flags of `serve`.
#[derive(Debug, Clone, Parser)]
#[command(name = "serve")]
pub(crate) struct ServeArgs {
    /// The address to listen on; 0.0.0.0:8080 opens it to every network, so add a layer of your own first
    /// [env: CICADA_LISTEN] [default: 127.0.0.1:8080]
    #[arg(long, value_name = "ADDRESS", value_parser = socket_address)]
    pub listen: Option<SocketAddr>,
    /// The settings of the node.
    #[command(flatten)]
    pub node: NodeArgs,
}

/// The flags of `worker`.
#[derive(Debug, Clone, Parser)]
#[command(name = "worker")]
pub(crate) struct WorkerArgs {
    /// The settings of the node.
    #[command(flatten)]
    pub node: NodeArgs,
    /// The base URL of a server's API, which selects HTTP mode [env: CICADA_SERVER]
    #[arg(long, value_name = "URL", help_heading = "HTTP")]
    pub server: Option<String>,
    /// The context of the config file, which selects HTTP mode [env: CICADA_CONTEXT]
    #[arg(long, value_name = "NAME", help_heading = "HTTP")]
    pub context: Option<String>,
    /// The bearer token, in HTTP mode only [env: CICADA_TOKEN]
    #[arg(long, value_name = "TOKEN", help_heading = "HTTP")]
    pub token: Option<String>,
}

/// The flags that `serve` and `worker` share.
#[derive(Debug, Clone, Default, Args)]
pub(crate) struct NodeArgs {
    /// Which store: a postgres:// or postgresql:// address, sqlite:FILE, or the path of a SQLite file. PostgreSQL that
    /// does not answer is tried again for about a minute; one try can take seconds, and a wrong password is tried again
    /// like an unreachable database [env: CICADA_STORE]
    #[arg(long, value_name = "STORE", help_heading = "Store")]
    pub store: Option<String>,
    /// The schema, on PostgreSQL [env: CICADA_STORE_SCHEMA] [default: public]
    #[arg(long, value_name = "SCHEMA", help_heading = "Store")]
    pub store_schema: Option<String>,
    /// The connections of this node, on PostgreSQL [env: CICADA_STORE_POOL_SIZE] [default: the larger of the store's
    /// default and the concurrency plus 4]
    #[arg(long, value_name = "N", value_parser = number::<u32>, help_heading = "Store")]
    pub store_pool_size: Option<u32>,
    /// How long outcomes are kept, which is also the window in which a message id is known; the same on every node
    /// [env: CICADA_RETENTION] [default: 7d]
    #[arg(long, value_name = "DURATION", value_parser = duration, help_heading = "Store")]
    pub retention: Option<Duration>,
    /// The types to host: all, or type names with commas; serve also takes none [env: CICADA_TYPES] [default: all]
    #[arg(long, value_name = "TYPES", help_heading = "Node")]
    pub types: Option<String>,
    /// The id of the node [env: CICADA_NODE_ID] [default: random]
    #[arg(long, value_name = "ID", value_parser = node_id, help_heading = "Node")]
    pub node_id: Option<NodeId>,
    /// A text shown with the node in the node table, such as the application and its version; kept as given, display
    /// only, not on a worker over HTTP [env: CICADA_NODE_LABEL] [default: none]
    #[arg(long, value_name = "TEXT", value_parser = node_label, help_heading = "Node")]
    pub node_label: Option<String>,
    /// The most turns that run at once [env: CICADA_CONCURRENCY] [default: 16]
    #[arg(long, value_name = "N", value_parser = number::<usize>, help_heading = "Node")]
    pub concurrency: Option<usize>,
    /// The lease of a turn [env: CICADA_LEASE_TTL] [default: 30s]
    #[arg(long, value_name = "DURATION", value_parser = duration, help_heading = "Node")]
    pub lease_ttl: Option<Duration>,
    /// How often a lease is renewed [env: CICADA_HEARTBEAT_INTERVAL] [default: a third of the lease]
    #[arg(long, value_name = "DURATION", value_parser = duration, help_heading = "Node")]
    pub heartbeat_interval: Option<Duration>,
    /// The longest sleep of the dispatch loop [env: CICADA_POLL_INTERVAL] [default: 500ms]
    #[arg(long, value_name = "DURATION", value_parser = duration, help_heading = "Node")]
    pub poll_interval: Option<Duration>,
    /// How long running turns may take to finish when the node stops [env: CICADA_SHUTDOWN_GRACE_PERIOD]
    /// [default: 30s]
    #[arg(long, value_name = "DURATION", value_parser = duration, help_heading = "Node")]
    pub shutdown_grace_period: Option<Duration>,
    /// The longest turn of a type that sets none [env: CICADA_MAX_TURN] [default: 15m]
    #[arg(long, value_name = "DURATION", value_parser = duration, help_heading = "Node")]
    pub max_turn: Option<Duration>,
    /// The claim limit: a number, or off [env: CICADA_MAX_LOST_CLAIMS] [default: 5]
    #[arg(long, value_name = "N|off", value_parser = lost_claims, help_heading = "Node")]
    pub max_lost_claims: Option<LostClaims>,
}

/// `--max-lost-claims`: a limit, or none (`off`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LostClaims(pub Option<u32>);

/// The command a node runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `serve`.
    Serve,
    /// `worker`.
    Worker,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Worker => "worker",
        }
    }
}

/// Which store, from the value of `--store` or `CICADA_STORE`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum StoreSpec {
    /// A SQLite file.
    Sqlite(PathBuf),
    /// A PostgreSQL address, which may hold a password.
    Postgres(String),
}

impl StoreSpec {
    /// The store a value names: `postgres://` and `postgresql://` are PostgreSQL, `sqlite:` (with or without `//`) a
    /// SQLite file named after it, and anything else that holds `://` or starts with `postgres:` or `postgresql:` is
    /// refused, so that a mistyped address never becomes the name of a file. Any other value is the path of a SQLite
    /// file. The error never holds the value, which may hold a password.
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if value.starts_with("postgres://") || value.starts_with("postgresql://") {
            return Ok(Self::Postgres(value.to_owned()));
        }
        if let Some(rest) = value.strip_prefix("sqlite:") {
            let file = rest.strip_prefix("//").unwrap_or(rest);
            return if file.is_empty() {
                Err("sqlite: names no file; give sqlite:FILE".to_owned())
            } else {
                Ok(Self::Sqlite(PathBuf::from(file)))
            };
        }
        if value.starts_with("postgres:") || value.starts_with("postgresql:") || value.contains("://") {
            return Err("not a store this binary knows; give a postgres:// or postgresql:// address, sqlite:FILE, or \
                        the path of a SQLite file (the value is not shown, as it may hold a password)"
                .to_owned());
        }
        Ok(Self::Sqlite(PathBuf::from(value)))
    }

    /// The backend of this store.
    pub(crate) fn backend(&self) -> Backend {
        match self {
            Self::Sqlite(_) => Backend::Sqlite,
            Self::Postgres(_) => Backend::Postgres,
        }
    }
}

impl fmt::Debug for StoreSpec {
    /// Without the address of PostgreSQL, which may hold a password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(path) => write!(f, "Sqlite({})", path.display()),
            Self::Postgres(_) => f.write_str("Postgres(…)"),
        }
    }
}

/// A store backend, each behind a feature of this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backend {
    /// The feature `sqlite`.
    Sqlite,
    /// The feature `postgres`.
    Postgres,
}

/// The backends this binary was built with.
pub(crate) fn built_in() -> Vec<Backend> {
    let mut backends = Vec::new();
    if cfg!(feature = "sqlite") {
        backends.push(Backend::Sqlite);
    }
    if cfg!(feature = "postgres") {
        backends.push(Backend::Postgres);
    }
    backends
}

/// Whether `store` can be opened by a binary with the backends `built`: an error that names the missing feature.
pub(crate) fn check_built_in(store: &StoreSpec, built: &[Backend]) -> Result<(), String> {
    let (name, feature) = match store.backend() {
        Backend::Sqlite => ("SQLite", "sqlite"),
        Backend::Postgres => ("PostgreSQL", "postgres"),
    };
    if built.contains(&store.backend()) {
        Ok(())
    } else {
        Err(format!("this binary was built without {name}: build it with the feature `{feature}` of cicada-app"))
    }
}

/// What `serve` or `worker` runs with.
#[derive(Debug, Clone)]
pub(crate) struct Settings<N = NodeConfig> {
    /// The store.
    pub store: StoreSpec,
    /// The schema, on PostgreSQL.
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    pub schema: String,
    /// The pool, on PostgreSQL, when it was given.
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    pub pool_size: Option<u32>,
    /// The retention of the store, when it was given.
    #[cfg_attr(not(any(feature = "sqlite", feature = "postgres")), allow(dead_code))]
    pub retention: Option<Duration>,
    /// The names of the types to host, in the order the binary hosts them.
    pub types: Vec<&'static str>,
    /// The parsed overrides or the settled configuration of the node.
    pub node: N,
    /// Where `serve` listens.
    pub listen: SocketAddr,
}

impl Settings {
    /// The pool on PostgreSQL: the one given, else the larger of the store's default and the concurrency plus 4.
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    pub(crate) fn pool_size(&self, store_default: u32) -> u32 {
        self.pool_size.unwrap_or_else(|| {
            let needed = u32::try_from(self.node.concurrency.saturating_add(4)).unwrap_or(u32::MAX);
            needed.max(store_default)
        })
    }
}

/// Settings for a node on the store, before its host config is known.
pub(crate) type ParsedSettings = Settings<NodeOverrides>;

impl ParsedSettings {
    /// Lays the parsed settings over the host's config or the default config of a node.
    pub(crate) fn apply(self, host: Option<NodeConfig>) -> Settings {
        let Self { store, schema, pool_size, retention, types, node, listen } = self;
        Settings { store, schema, pool_size, retention, types, node: node.apply_to_node(host), listen }
    }
}

/// Parses the flags and variables of `kind` and selects the types it will host.
/// An error names the flag or the variable that cannot be used.
pub(crate) fn parse(
    kind: Kind,
    args: &NodeArgs,
    listen: Option<SocketAddr>,
    env: &Env,
    hosted: &[&'static str],
) -> Result<ParsedSettings, String> {
    let store = match (&args.store, variable(env, "CICADA_STORE")) {
        (Some(value), _) => StoreSpec::parse(value).map_err(|e| format!("--store: {e}"))?,
        (None, Some(value)) => StoreSpec::parse(value).map_err(|e| format!("CICADA_STORE: {e}"))?,
        (None, None) => {
            return Err(format!("{} needs a store: pass --store or set CICADA_STORE", kind.name()));
        }
    };
    let types = parse_types(kind, args, env, hosted)?;
    let node = NodeOverrides::parse(args, env, WorkerMode::Store)?;
    let default_listen = DEFAULT_LISTEN.parse().expect("a socket address");
    Ok(Settings {
        store,
        schema: setting(args.store_schema.clone(), env, "CICADA_STORE_SCHEMA", |v| Ok(v.to_owned()))?
            .unwrap_or_else(|| "public".to_owned()),
        pool_size: setting(args.store_pool_size, env, "CICADA_STORE_POOL_SIZE", number)?,
        retention: setting(args.retention, env, "CICADA_RETENTION", duration)?,
        types,
        node,
        listen: setting(listen, env, "CICADA_LISTEN", socket_address)?.unwrap_or(default_listen),
    })
}

/// Where a worker runs its turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerMode {
    /// On the store, through an `ActorSystem`.
    Store,
    /// Through a server's HTTP API, with a `Worker`.
    Http,
}

/// The mode of a worker, from its flags and the variables alone: `--store` selects the store, `--server` or `--context`
/// HTTP, and both are an error. Without them a non-blank `CICADA_STORE` selects the store, else a non-blank
/// `CICADA_SERVER` or `CICADA_CONTEXT` HTTP. A token or the current context of the config file alone selects nothing,
/// and `--token` on the store is an error. An error is a command line that cannot be used.
pub(crate) fn worker_mode(args: &WorkerArgs, env: &Env) -> Result<WorkerMode, String> {
    let http_flag = match (&args.server, &args.context) {
        (Some(_), _) => Some("--server"),
        (None, Some(_)) => Some("--context"),
        (None, None) => None,
    };
    let mode = match (args.node.store.is_some(), http_flag) {
        (true, Some(flag)) => {
            return Err(format!(
                "worker takes --store or {flag}, not both: --store runs the turns on the store, {flag} through a \
                 server's HTTP API"
            ));
        }
        (true, None) => WorkerMode::Store,
        (false, Some(_)) => WorkerMode::Http,
        (false, None) if variable(env, "CICADA_STORE").is_some() => WorkerMode::Store,
        (false, None) if variable(env, "CICADA_SERVER").is_some() || variable(env, "CICADA_CONTEXT").is_some() => {
            WorkerMode::Http
        }
        (false, None) => {
            return Err("worker needs a store or a server: pass --store or set CICADA_STORE to run on the store, or \
                        pass --server or --context, or set CICADA_SERVER or CICADA_CONTEXT, to run through a server's \
                        HTTP API"
                .to_owned());
        }
    };
    if mode == WorkerMode::Store && args.token.is_some() {
        return Err("--token is for a worker over HTTP: add --server or --context, or leave --token out on the store"
            .to_owned());
    }
    Ok(mode)
}

/// What a worker over HTTP runs with: the types and the configuration of its `Worker`. The server comes from
/// [`http_target`].
#[derive(Debug, Clone)]
pub(crate) struct HttpSettings<W = WorkerConfig> {
    /// The names of the types to host, in the order the binary hosts them.
    pub types: Vec<&'static str>,
    /// The parsed overrides or the settled configuration of the worker.
    pub worker: W,
}

/// Settings for a worker over HTTP, before its host config is known.
pub(crate) type ParsedHttpSettings = HttpSettings<NodeOverrides>;

impl ParsedHttpSettings {
    /// Lays the parsed settings over the worker fields of the host config or the default config of a worker.
    pub(crate) fn apply(self, host: Option<NodeConfig>) -> HttpSettings {
        HttpSettings { types: self.types, worker: self.worker.apply_to_worker(host) }
    }
}

/// Parses the flags and variables for a worker over HTTP and selects the types it will host.
/// It refuses flags for settings the server owns and skips their variables and `CICADA_LISTEN`.
pub(crate) fn parse_http(args: &NodeArgs, env: &Env, hosted: &[&'static str]) -> Result<ParsedHttpSettings, String> {
    let refused: Vec<&str> = [
        (args.store_schema.is_some(), "--store-schema"),
        (args.store_pool_size.is_some(), "--store-pool-size"),
        (args.retention.is_some(), "--retention"),
        (args.lease_ttl.is_some(), "--lease-ttl"),
        (args.heartbeat_interval.is_some(), "--heartbeat-interval"),
        (args.max_lost_claims.is_some(), "--max-lost-claims"),
    ]
    .into_iter()
    .filter_map(|(given, flag)| given.then_some(flag))
    .collect();
    if !refused.is_empty() {
        return Err(format!(
            "a worker over HTTP does not take {}: the server owns the store and the leases",
            refused.join(", ")
        ));
    }
    if args.node_label.is_some() {
        return Err("a worker over HTTP does not take --node-label: it has no row in the node table".to_owned());
    }
    let types = parse_types(Kind::Worker, args, env, hosted)?;
    let worker = NodeOverrides::parse(args, env, WorkerMode::Http)?;
    Ok(HttpSettings { types, worker })
}

/// The node fields supplied by flags or variables, before falling back to a host config.
#[derive(Debug, Clone)]
pub(crate) struct NodeOverrides {
    id: Option<NodeId>,
    concurrency: Option<usize>,
    lease_ttl: Option<Duration>,
    heartbeat_interval: Option<Duration>,
    poll_interval: Option<Duration>,
    grace_period: Option<Duration>,
    max_turn: Option<Duration>,
    max_lost_claims: Option<LostClaims>,
    label: Option<String>,
}

impl NodeOverrides {
    fn parse(args: &NodeArgs, env: &Env, mode: WorkerMode) -> Result<Self, String> {
        let id = setting(args.node_id.clone(), env, "CICADA_NODE_ID", node_id)?;
        let concurrency = setting(args.concurrency, env, "CICADA_CONCURRENCY", number)?;
        let (lease_ttl, heartbeat_interval) = if mode == WorkerMode::Store {
            (
                setting(args.lease_ttl, env, "CICADA_LEASE_TTL", duration)?,
                setting(args.heartbeat_interval, env, "CICADA_HEARTBEAT_INTERVAL", duration)?,
            )
        } else {
            (None, None)
        };
        let poll_interval = setting(args.poll_interval, env, "CICADA_POLL_INTERVAL", duration)?;
        let grace_period = setting(args.shutdown_grace_period, env, "CICADA_SHUTDOWN_GRACE_PERIOD", duration)?;
        let max_turn = setting(args.max_turn, env, "CICADA_MAX_TURN", duration)?;
        let (max_lost_claims, label) = if mode == WorkerMode::Store {
            let max_lost_claims = setting(args.max_lost_claims, env, "CICADA_MAX_LOST_CLAIMS", lost_claims)?;
            // An empty label is unset, a blank one is an error and outer spaces are kept.
            let label = match (&args.node_label, env.get("CICADA_NODE_LABEL").filter(|value| !value.is_empty())) {
                (Some(label), _) => Some(label.clone()),
                (None, Some(value)) => {
                    Some(node_label(value).map_err(|e| format!("CICADA_NODE_LABEL: invalid value: {e}"))?)
                }
                (None, None) => None,
            };
            (max_lost_claims, label)
        } else {
            (None, None)
        };
        Ok(Self {
            id,
            concurrency,
            lease_ttl,
            heartbeat_interval,
            poll_interval,
            grace_period,
            max_turn,
            max_lost_claims,
            label,
        })
    }

    fn apply_to_node(self, host: Option<NodeConfig>) -> NodeConfig {
        let node = host.unwrap_or_default();
        NodeConfig {
            node_id: self.id.unwrap_or(node.node_id),
            concurrency: self.concurrency.unwrap_or(node.concurrency),
            lease_ttl: self.lease_ttl.unwrap_or(node.lease_ttl),
            heartbeat_interval: self.heartbeat_interval.or(node.heartbeat_interval),
            poll_interval: self.poll_interval.unwrap_or(node.poll_interval),
            shutdown_grace_period: self.grace_period.unwrap_or(node.shutdown_grace_period),
            default_max_turn: self.max_turn.unwrap_or(node.default_max_turn),
            max_lost_claims: self.max_lost_claims.map_or(node.max_lost_claims, |limit| limit.0),
            label: self.label.or(node.label),
            ..node
        }
    }

    fn apply_to_worker(self, host: Option<NodeConfig>) -> WorkerConfig {
        let worker = host.map_or_else(WorkerConfig::default, |host| WorkerConfig {
            worker_id: host.node_id,
            concurrency: host.concurrency,
            poll_interval: host.poll_interval,
            default_max_turn: host.default_max_turn,
            shutdown_grace_period: host.shutdown_grace_period,
            shutdown_cancel_timeout: host.shutdown_cancel_timeout,
        });
        WorkerConfig {
            worker_id: self.id.unwrap_or(worker.worker_id),
            concurrency: self.concurrency.unwrap_or(worker.concurrency),
            poll_interval: self.poll_interval.unwrap_or(worker.poll_interval),
            shutdown_grace_period: self.grace_period.unwrap_or(worker.shutdown_grace_period),
            default_max_turn: self.max_turn.unwrap_or(worker.default_max_turn),
            ..worker
        }
    }
}

/// Selects the types by flag, then environment variable, with all the binary's types as the default.
fn parse_types(kind: Kind, args: &NodeArgs, env: &Env, hosted: &[&'static str]) -> Result<Vec<&'static str>, String> {
    match (&args.types, variable(env, "CICADA_TYPES")) {
        (Some(value), _) => pick_types(kind, value, hosted).map_err(|e| format!("--types: {e}")),
        (None, Some(value)) => pick_types(kind, value, hosted).map_err(|e| format!("CICADA_TYPES: {e}")),
        (None, None) => Ok(hosted.to_vec()),
    }
}

/// The server of a worker over HTTP, with its token and headers, as the operator commands resolve them: each from its
/// flag, else its variable, else the context of the config file (which may give its token and headers when `--server`
/// overrides its URL). A blank `CICADA_SERVER` or `CICADA_CONTEXT` counts as unset. `bin` is the binary as it was
/// called, for the hint. Reads the config file; an error is a failure of the command.
pub(crate) fn http_target(args: &WorkerArgs, env: &Env, bin: &str) -> anyhow::Result<Target> {
    let mut env = env.clone();
    for name in ["CICADA_SERVER", "CICADA_CONTEXT"] {
        if variable(&env, name).is_none() {
            env.remove(name);
        }
    }
    let flags = Overrides { context: args.context.clone(), server: args.server.clone(), token: args.token.clone() };
    config::resolve(&flags, &env, &Config::load(&config::path(&env)?)?, bin)
}

/// The value of variable `name`, trimmed; `None` when it is unset or blank.
fn variable<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    env.get(name).map(|value| value.trim()).filter(|value| !value.is_empty())
}

/// The flag's value, else the variable `name` read with `parse`, whose error names the variable.
fn setting<T>(
    flag: Option<T>,
    env: &Env,
    name: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    if flag.is_some() {
        return Ok(flag);
    }
    variable(env, name)
        .map(|value| parse(value).map_err(|e| format!("{name}: invalid value '{value}': {e}")))
        .transpose()
}

/// The types that `value` picks out of `hosted`, in the order of `hosted`. `all` and `none` are keywords only when they
/// are the whole value; in a list every name is a type name.
fn pick_types(kind: Kind, value: &str, hosted: &[&'static str]) -> Result<Vec<&'static str>, String> {
    match value.trim() {
        "all" => return Ok(hosted.to_vec()),
        "none" if kind == Kind::Serve => return Ok(Vec::new()),
        "none" => return Err("a worker must host a type; none is for serve, a node that only serves HTTP".to_owned()),
        _ => {}
    }
    let names: Vec<&str> = value.split(',').map(str::trim).collect();
    if names.iter().any(|name| name.is_empty()) {
        return Err(format!("an empty name in '{value}'; this binary hosts {}", hosted.join(", ")));
    }
    let unknown: Vec<&str> = names.iter().copied().filter(|name| !hosted.contains(name)).collect();
    if !unknown.is_empty() {
        return Err(format!("this binary does not host {}; it hosts {}", unknown.join(", "), hosted.join(", ")));
    }
    Ok(hosted.iter().copied().filter(|name| names.contains(name)).collect())
}

/// A duration: a whole number with `ms`, `s`, `m`, `h` or `d` (`250ms`, `5s`, `7d`), or ISO 8601 (`PT5S`, `P7D`).
pub(crate) fn duration(text: &str) -> Result<Duration, String> {
    let expected = || "expected a duration such as 250ms, 5s, 7d or PT5S".to_owned();
    let text = text.trim();
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    if digits == 0 {
        return cicada::time::parse_duration(text).map_err(|_| expected());
    }
    let (number, unit) = text.split_at(digits);
    let number: u64 = number.parse().map_err(|_| expected())?;
    let seconds = |factor: u64| number.checked_mul(factor).map(Duration::from_secs);
    match unit {
        "ms" => Some(Duration::from_millis(number)),
        "s" => seconds(1),
        "m" => seconds(60),
        "h" => seconds(3600),
        "d" => seconds(86_400),
        _ => None,
    }
    .ok_or_else(expected)
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
    text.trim().parse().map_err(|_| "expected a whole number".to_owned())
}

fn node_id(text: &str) -> Result<NodeId, String> {
    NodeId::new(text.trim()).map_err(|e| e.to_string())
}

/// A label as `NodeConfig` takes it, kept as given: not blank, no NUL, at most 200 characters.
fn node_label(text: &str) -> Result<String, String> {
    if text.trim().is_empty() {
        Err("a label must not be blank".to_owned())
    } else if text.contains('\0') {
        Err("a label must not contain NUL".to_owned())
    } else if text.chars().count() > cicada::runtime::limits::MAX_ID_LENGTH {
        Err(format!("a label must be at most {} characters long", cicada::runtime::limits::MAX_ID_LENGTH))
    } else {
        Ok(text.to_owned())
    }
}

fn lost_claims(text: &str) -> Result<LostClaims, String> {
    match text.trim() {
        "off" => Ok(LostClaims(None)),
        n => n.parse().map(|n| LostClaims(Some(n))).map_err(|_| "expected a whole number, or off".to_owned()),
    }
}

fn socket_address(text: &str) -> Result<SocketAddr, String> {
    text.trim().parse().map_err(|_| "expected an address such as 127.0.0.1:8080 or 0.0.0.0:8080".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTED: [&str; 2] = ["a", "b"];

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    /// The settings of `kind` from the command line `flags` (after the command) and `env`, for a binary with `hosted`.
    fn read(kind: Kind, flags: &[&str], env: &Env, host: Option<&NodeConfig>, hosted: &[&'static str]) -> Settings {
        try_read(kind, flags, env, host, hosted).unwrap_or_else(|e| panic!("{flags:?}: {e}"))
    }

    fn try_read(
        kind: Kind,
        flags: &[&str],
        env: &Env,
        host: Option<&NodeConfig>,
        hosted: &[&'static str],
    ) -> Result<Settings, String> {
        let (args, listen) = match kind {
            Kind::Serve => {
                let serve = ServeArgs::try_parse_from(std::iter::once("serve").chain(flags.iter().copied()))
                    .map_err(|e| e.to_string())?;
                (serve.node, serve.listen)
            }
            Kind::Worker => {
                let worker = WorkerArgs::try_parse_from(std::iter::once("worker").chain(flags.iter().copied()))
                    .map_err(|e| e.to_string())?;
                (worker.node, None)
            }
        };
        Ok(parse(kind, &args, listen, env, hosted)?.apply(host.cloned()))
    }

    fn serve(flags: &[&str]) -> Settings {
        read(Kind::Serve, flags, &Env::new(), None, &HOSTED)
    }

    fn serve_fails(flags: &[&str], env: &Env) -> String {
        try_read(Kind::Serve, flags, env, None, &HOSTED).expect_err("an error")
    }

    #[test]
    fn the_flags_are_consistent() {
        use clap::CommandFactory;
        ServeArgs::command().debug_assert();
        WorkerArgs::command().debug_assert();
    }

    /// Scenario: A SQLite file.
    #[test]
    fn a_path_is_a_sqlite_file() {
        assert_eq!(serve(&["--store", "actors.db"]).store, StoreSpec::Sqlite("actors.db".into()));
        assert_eq!(serve(&["--store", "/data/actors.db"]).store, StoreSpec::Sqlite("/data/actors.db".into()));
    }

    /// Scenario: PostgreSQL from a variable.
    #[test]
    fn a_postgres_address_from_the_variable_is_postgres_in_the_schema_public() {
        let url = "postgres://user:secret@db:5432/actors";
        let settings = read(Kind::Worker, &[], &env(&[("CICADA_STORE", url)]), None, &HOSTED);
        assert_eq!(settings.store, StoreSpec::Postgres(url.to_owned()));
        assert_eq!(settings.schema, "public");
        let other = read(Kind::Worker, &[], &env(&[("CICADA_STORE", "postgresql://db/x")]), None, &HOSTED);
        assert_eq!(other.store.backend(), Backend::Postgres);
        assert!(!format!("{other:?}").contains("db/x"), "Debug leaves the address out");
    }

    /// Scenario: A flag wins. A blank variable counts as unset.
    #[test]
    fn a_flag_wins_over_its_variable_and_a_blank_variable_is_unset() {
        let e = env(&[("CICADA_LISTEN", "127.0.0.1:9000"), ("CICADA_STORE", "a.db")]);
        assert_eq!(read(Kind::Serve, &["--listen", "127.0.0.1:9100"], &e, None, &HOSTED).listen.port(), 9100);
        assert_eq!(read(Kind::Serve, &[], &e, None, &HOSTED).listen.port(), 9000);
        let store = read(Kind::Serve, &["--store", "b.db"], &e, None, &HOSTED).store;
        assert_eq!(store, StoreSpec::Sqlite("b.db".into()));
        let blank = env(&[("CICADA_LISTEN", "  "), ("CICADA_STORE", "a.db"), ("CICADA_CONCURRENCY", "")]);
        let settings = read(Kind::Serve, &[], &blank, None, &HOSTED);
        assert_eq!((settings.listen.to_string().as_str(), settings.node.concurrency), (DEFAULT_LISTEN, 16));
    }

    /// Scenario: No store.
    #[test]
    fn without_a_store_the_error_names_both_ways_to_give_one() {
        let error = serve_fails(&[], &Env::new());
        assert!(error.contains("--store") && error.contains("CICADA_STORE"), "{error}");
    }

    /// Scenario: An address of another kind.
    #[test]
    fn an_address_of_another_kind_is_refused_without_its_value() {
        let error = serve_fails(&["--store", "postgress://user:secret@host/db"], &Env::new());
        assert!(
            error.starts_with("--store: ") && error.contains("postgres://") && error.contains("sqlite:"),
            "{error}"
        );
        assert!(!error.contains("secret") && !error.contains("host/db"), "{error}");
        let from_env = serve_fails(&[], &env(&[("CICADA_STORE", "mysql://user:secret@host/db")]));
        assert!(from_env.starts_with("CICADA_STORE: ") && !from_env.contains("secret"), "{from_env}");
    }

    /// Scenario: A PostgreSQL address without its slashes.
    #[test]
    fn a_postgres_address_without_its_slashes_is_refused() {
        for value in ["postgres:db", "postgresql:user:secret@db", "postgres:"] {
            let error = serve_fails(&["--store", value], &Env::new());
            assert!(error.contains("postgres://") && !error.contains("secret"), "{value}: {error}");
        }
    }

    /// Scenario: A SQLite address.
    #[test]
    fn a_sqlite_address_names_its_file_with_or_without_slashes() {
        for value in ["sqlite:actors.db", "sqlite://actors.db"] {
            assert_eq!(serve(&["--store", value]).store, StoreSpec::Sqlite("actors.db".into()), "{value}");
        }
        assert_eq!(serve(&["--store", "sqlite:///data/a.db"]).store, StoreSpec::Sqlite("/data/a.db".into()));
        assert!(serve_fails(&["--store", "sqlite:"], &Env::new()).contains("names no file"));
    }

    /// Scenario: A backend that is not built in.
    #[test]
    fn a_backend_that_is_not_built_in_names_its_feature() {
        let postgres = StoreSpec::Postgres("postgres://db/x".into());
        let error = check_built_in(&postgres, &[Backend::Sqlite]).unwrap_err();
        assert!(error.contains("without PostgreSQL") && error.contains("feature `postgres`"), "{error}");
        let error = check_built_in(&StoreSpec::Sqlite("a.db".into()), &[]).unwrap_err();
        assert!(error.contains("without SQLite") && error.contains("feature `sqlite`"), "{error}");
        assert!(check_built_in(&postgres, &[Backend::Sqlite, Backend::Postgres]).is_ok());
    }

    /// Scenario: The retention.
    #[test]
    fn the_retention_takes_hours_and_days() {
        assert_eq!(serve(&["--store", "a.db", "--retention", "1h"]).retention, Some(Duration::from_secs(3600)));
        assert_eq!(serve(&["--store", "a.db", "--retention", "14d"]).retention, Some(Duration::from_secs(14 * 86_400)));
        assert_eq!(serve(&["--store", "a.db"]).retention, None, "the store's default");
    }

    /// Scenario: The pool.
    #[test]
    fn the_pool_is_the_given_one_or_room_for_the_concurrency() {
        let pg = ["--store", "postgres://db/x"];
        assert_eq!(serve(&[&pg[..], &["--store-pool-size", "5"]].concat()).pool_size(20), 5);
        assert_eq!(serve(&[&pg[..], &["--concurrency", "40"]].concat()).pool_size(20), 44);
        assert_eq!(serve(&[&pg[..], &["--concurrency", "4"]].concat()).pool_size(20), 20, "the store's default");
    }

    /// Scenario: The default address.
    #[test]
    fn serve_listens_on_the_loopback_address_by_default() {
        assert_eq!(serve(&["--store", "a.db"]).listen, "127.0.0.1:8080".parse::<SocketAddr>().unwrap());
    }

    /// Scenario: A variable over the host's value.
    #[test]
    fn a_variable_wins_over_the_hosts_node_config() {
        let host = NodeConfig { concurrency: 4, ..NodeConfig::default() };
        let e = env(&[("CICADA_STORE", "a.db"), ("CICADA_CONCURRENCY", "8")]);
        assert_eq!(read(Kind::Serve, &[], &e, Some(&host), &HOSTED).node.concurrency, 8);
        assert_eq!(read(Kind::Serve, &["--concurrency", "12"], &e, Some(&host), &HOSTED).node.concurrency, 12);
    }

    /// Scenario: The host's value over the default.
    #[test]
    fn the_hosts_node_config_wins_over_the_default() {
        let host = NodeConfig {
            concurrency: 4,
            node_id: NodeId::new("host-node").unwrap(),
            max_lost_claims: None,
            ..NodeConfig::default()
        };
        let settings = read(Kind::Serve, &["--store", "a.db"], &Env::new(), Some(&host), &HOSTED);
        assert_eq!(settings.node.concurrency, 4);
        assert_eq!(settings.node.node_id.as_str(), "host-node");
        assert_eq!(settings.node.max_lost_claims, None);
    }

    /// Every flag and variable of the node sets its field.
    #[test]
    fn every_setting_of_the_node_has_a_flag_and_a_variable() {
        let flags = [
            "--store",
            "a.db",
            "--node-id",
            "n1",
            "--lease-ttl",
            "3s",
            "--heartbeat-interval",
            "PT1S",
            "--poll-interval",
            "250ms",
            "--shutdown-grace-period",
            "2m",
            "--max-turn",
            "1h",
            "--max-lost-claims",
            "7",
        ];
        let node = serve(&flags).node;
        assert_eq!(node.node_id.as_str(), "n1");
        assert_eq!(node.lease_ttl, Duration::from_secs(3));
        assert_eq!(node.heartbeat_interval, Some(Duration::from_secs(1)));
        assert_eq!(node.poll_interval, Duration::from_millis(250));
        assert_eq!(node.shutdown_grace_period, Duration::from_secs(120));
        assert_eq!(node.default_max_turn, Duration::from_secs(3600));
        assert_eq!(node.max_lost_claims, Some(7));
        let e = env(&[
            ("CICADA_STORE", "a.db"),
            ("CICADA_NODE_ID", "n2"),
            ("CICADA_LEASE_TTL", "6s"),
            ("CICADA_HEARTBEAT_INTERVAL", "2s"),
            ("CICADA_POLL_INTERVAL", "100ms"),
            ("CICADA_SHUTDOWN_GRACE_PERIOD", "3s"),
            ("CICADA_MAX_TURN", "P1D"),
            ("CICADA_MAX_LOST_CLAIMS", "off"),
            ("CICADA_STORE_SCHEMA", "orders"),
            ("CICADA_STORE_POOL_SIZE", "9"),
            ("CICADA_RETENTION", "P7D"),
        ]);
        let settings = read(Kind::Worker, &[], &e, None, &HOSTED);
        let node = &settings.node;
        assert_eq!(node.node_id.as_str(), "n2");
        assert_eq!(node.lease_ttl, Duration::from_secs(6));
        assert_eq!(node.heartbeat_interval, Some(Duration::from_secs(2)));
        assert_eq!(node.poll_interval, Duration::from_millis(100));
        assert_eq!(node.shutdown_grace_period, Duration::from_secs(3));
        assert_eq!(node.default_max_turn, Duration::from_secs(86_400));
        assert_eq!(node.max_lost_claims, None);
        assert_eq!((settings.schema.as_str(), settings.pool_size), ("orders", Some(9)));
        assert_eq!(settings.retention, Some(Duration::from_secs(7 * 86_400)));
    }

    /// Scenario: The limit of lost claims is turned off.
    #[test]
    fn the_limit_of_lost_claims_is_turned_off_with_off() {
        assert_eq!(serve(&["--store", "a.db", "--max-lost-claims", "off"]).node.max_lost_claims, None);
    }

    /// Scenario: A value that cannot be read.
    #[test]
    fn a_variable_that_cannot_be_read_is_named() {
        let error = serve_fails(&["--store", "a.db"], &env(&[("CICADA_LEASE_TTL", "soon")]));
        assert!(error.starts_with("CICADA_LEASE_TTL: invalid value 'soon'"), "{error}");
        let error = serve_fails(&["--store", "a.db"], &env(&[("CICADA_MAX_LOST_CLAIMS", "never")]));
        assert!(error.starts_with("CICADA_MAX_LOST_CLAIMS"), "{error}");
        let error = serve_fails(&["--store", "a.db"], &env(&[("CICADA_LISTEN", "localhost")]));
        assert!(error.starts_with("CICADA_LISTEN"), "{error}");
        let flag = serve_fails(&["--store", "a.db", "--lease-ttl", "soon"], &Env::new());
        assert!(flag.contains("'--lease-ttl <DURATION>'"), "{flag}");
    }

    #[test]
    fn durations_take_a_unit_or_iso_8601() {
        assert_eq!(duration("250ms"), Ok(Duration::from_millis(250)));
        assert_eq!(duration("5s"), Ok(Duration::from_secs(5)));
        assert_eq!(duration("2m"), Ok(Duration::from_secs(120)));
        assert_eq!(duration("7d"), Ok(Duration::from_secs(7 * 86_400)));
        assert_eq!(duration("PT0.5S"), Ok(Duration::from_millis(500)));
        assert_eq!(duration("P7D"), Ok(Duration::from_secs(7 * 86_400)));
        for bad in ["soon", "5", "5x", "-5s", "", "99999999999999999999s"] {
            assert!(duration(bad).is_err(), "{bad}");
        }
    }

    /// Scenario: A list.
    #[test]
    fn a_list_hosts_the_types_it_names() {
        assert_eq!(read(Kind::Worker, &["--store", "a.db", "--types", "a"], &Env::new(), None, &HOSTED).types, ["a"]);
        let e = env(&[("CICADA_STORE", "a.db"), ("CICADA_TYPES", "b, a")]);
        assert_eq!(read(Kind::Worker, &[], &e, None, &HOSTED).types, ["a", "b"], "in the order of the binary");
        assert_eq!(read(Kind::Worker, &["--store", "a.db"], &Env::new(), None, &HOSTED).types, ["a", "b"]);
    }

    /// Scenario: None.
    #[test]
    fn serve_takes_none() {
        assert!(serve(&["--store", "a.db", "--types", "none"]).types.is_empty());
    }

    /// Scenario: A type the binary does not have.
    #[test]
    fn a_type_the_binary_does_not_host_is_named_with_the_hosted_ones() {
        let error = serve_fails(&["--store", "a.db", "--types", "a,nope"], &Env::new());
        assert_eq!(error, "--types: this binary does not host nope; it hosts a, b");
    }

    /// Scenario: An empty name and a name twice.
    #[test]
    fn an_empty_name_is_refused_and_a_name_twice_counts_once() {
        let error = try_read(Kind::Worker, &["--store", "a.db", "--types", "a,,b"], &Env::new(), None, &HOSTED);
        assert_eq!(error.unwrap_err(), "--types: an empty name in 'a,,b'; this binary hosts a, b");
        assert_eq!(read(Kind::Worker, &["--store", "a.db", "--types", "a,a"], &Env::new(), None, &HOSTED).types, ["a"]);
    }

    /// Scenario: A type called `all`.
    #[test]
    fn all_is_a_keyword_only_as_the_whole_value() {
        let hosted = ["all", "b"];
        for value in ["all,b", "all"] {
            let types = read(Kind::Worker, &["--store", "a.db", "--types", value], &Env::new(), None, &hosted).types;
            assert_eq!(types, ["all", "b"], "{value}");
        }
        let error = try_read(Kind::Worker, &["--store", "a.db", "--types", "all,b"], &Env::new(), None, &HOSTED);
        assert!(error.unwrap_err().contains("does not host all"), "a list names types only");
    }

    /// Scenario: A worker that hosts nothing.
    #[test]
    fn a_worker_must_host_a_type() {
        let error = try_read(Kind::Worker, &["--store", "a.db", "--types", "none"], &Env::new(), None, &HOSTED);
        assert!(error.unwrap_err().contains("a worker must host a type"));
    }

    /// Scenario: Label precedence.
    #[test]
    fn the_label_comes_from_the_flag_the_raw_variable_the_host_or_nothing() {
        let host = NodeConfig { label: Some("host".into()), ..NodeConfig::default() };
        let store = ("CICADA_STORE", "a.db");
        let label =
            |flags: &[&str], e: &Env, host: Option<&NodeConfig>| read(Kind::Serve, flags, e, host, &HOSTED).node.label;
        let with_env = env(&[store, ("CICADA_NODE_LABEL", "  from env 🦀 ")]);
        assert_eq!(label(&["--node-label", " flag "], &with_env, Some(&host)).as_deref(), Some(" flag "));
        assert_eq!(label(&[], &with_env, Some(&host)).as_deref(), Some("  from env 🦀 "), "outer spaces kept");
        assert_eq!(label(&[], &env(&[store]), Some(&host)).as_deref(), Some("host"));
        assert_eq!(label(&[], &env(&[store, ("CICADA_NODE_LABEL", "")]), Some(&host)).as_deref(), Some("host"));
        assert_eq!(label(&[], &env(&[store]), None), None);
        let worker = read(Kind::Worker, &["--node-label", "w"], &env(&[store]), None, &HOSTED).node.label;
        assert_eq!(worker.as_deref(), Some("w"), "a worker on the store takes it too");
        assert_eq!(label(&["--node-label", &"é".repeat(200)], &env(&[store]), None), Some("é".repeat(200)));
    }

    /// Scenario: Label precedence (values that cannot be used).
    #[test]
    fn a_blank_a_nul_or_a_too_long_label_is_a_usage_error_and_the_host_s_is_left_to_start() {
        let store = ("CICADA_STORE", "a.db");
        for bad in [" \t ", "a\0b", &"x".repeat(201)] {
            let flag = try_read(Kind::Serve, &["--node-label", bad], &env(&[store]), None, &HOSTED).unwrap_err();
            assert!(flag.contains("--node-label"), "{flag}");
            let variable = try_read(Kind::Serve, &[], &env(&[store, ("CICADA_NODE_LABEL", bad)]), None, &HOSTED);
            let variable = variable.unwrap_err();
            assert!(variable.starts_with("CICADA_NODE_LABEL: invalid value: a label must"), "{variable}");
        }
        assert!(try_read(Kind::Serve, &["--node-label", ""], &env(&[store]), None, &HOSTED).is_err());
        let host = NodeConfig { label: Some(" ".into()), ..NodeConfig::default() };
        let settings = read(Kind::Serve, &[], &env(&[store]), Some(&host), &HOSTED);
        assert_eq!(settings.node.label.as_deref(), Some(" "), "the start of the node refuses it");
    }

    // ---- a worker over HTTP ----

    fn worker_args(flags: &[&str]) -> WorkerArgs {
        WorkerArgs::try_parse_from(std::iter::once("worker").chain(flags.iter().copied()))
            .unwrap_or_else(|e| panic!("{flags:?}: {e}"))
    }

    fn mode(flags: &[&str], env: &Env) -> Result<WorkerMode, String> {
        worker_mode(&worker_args(flags), env)
    }

    fn http(flags: &[&str], env: &Env, host: Option<&NodeConfig>) -> Result<HttpSettings, String> {
        Ok(parse_http(&worker_args(flags).node, env, &HOSTED)?.apply(host.cloned()))
    }

    /// Scenario: Explicit selection overrides ambient settings.
    #[test]
    fn an_explicit_mode_flag_wins_over_the_variables_and_the_two_cannot_be_combined() {
        let ambient = env(&[("CICADA_STORE", "mysql://not-a-store"), ("CICADA_SERVER", "http://env")]);
        assert_eq!(mode(&["--server", "http://x"], &ambient), Ok(WorkerMode::Http), "the store value is not parsed");
        assert_eq!(mode(&["--context", "prod"], &ambient), Ok(WorkerMode::Http));
        let http_env = env(&[("CICADA_SERVER", "http://env"), ("CICADA_CONTEXT", "prod"), ("CICADA_TOKEN", "t")]);
        assert_eq!(mode(&["--store", "a.db"], &http_env), Ok(WorkerMode::Store));
        for flag in ["--server", "--context"] {
            let error = mode(&["--store", "a.db", flag, "x"], &Env::new()).unwrap_err();
            assert!(error.starts_with(&format!("worker takes --store or {flag}, not both")), "{error}");
        }
    }

    /// Scenario: Existing deployments with operator variables.
    #[test]
    fn without_a_mode_flag_the_store_variable_wins_and_blank_variables_are_unset() {
        let both = env(&[("CICADA_STORE", "a.db"), ("CICADA_SERVER", "http://x"), ("CICADA_CONTEXT", "prod")]);
        assert_eq!(mode(&[], &both), Ok(WorkerMode::Store));
        assert_eq!(mode(&[], &env(&[("CICADA_SERVER", "http://x")])), Ok(WorkerMode::Http));
        assert_eq!(mode(&[], &env(&[("CICADA_CONTEXT", "prod")])), Ok(WorkerMode::Http));
        let blank_store = env(&[("CICADA_STORE", "  "), ("CICADA_SERVER", "http://x")]);
        assert_eq!(mode(&[], &blank_store), Ok(WorkerMode::Http));
        let blank_server = env(&[("CICADA_SERVER", " "), ("CICADA_CONTEXT", "\t")]);
        assert!(mode(&[], &blank_server).is_err(), "whitespace selects nothing");
    }

    /// Scenario: Context and token alone.
    #[test]
    fn a_token_alone_selects_nothing_and_a_token_flag_needs_http() {
        for (flags, e) in [
            (&[][..], env(&[("CICADA_TOKEN", "t")])),
            (&["--token", "t"][..], Env::new()),
            (&[][..], env(&[("CICADA_CONFIG", "/some/config.toml")])),
        ] {
            let error = mode(flags, &e).unwrap_err();
            assert!(error.starts_with("worker needs a store or a server"), "{flags:?}: {error}");
            for named in ["--store", "CICADA_STORE", "--server", "--context", "CICADA_SERVER", "CICADA_CONTEXT"] {
                assert!(error.contains(named), "{error}");
            }
        }
        let error = mode(&["--token", "t"], &env(&[("CICADA_STORE", "a.db")])).unwrap_err();
        assert!(error.starts_with("--token is for a worker over HTTP"), "{error}");
        let error = mode(&["--store", "a.db", "--token", "t"], &Env::new()).unwrap_err();
        assert!(error.starts_with("--token is for a worker over HTTP"), "{error}");
        assert_eq!(mode(&[], &env(&[("CICADA_STORE", "a.db"), ("CICADA_TOKEN", "t")])), Ok(WorkerMode::Store));
    }

    /// Scenario: Applicable settings.
    #[test]
    fn the_settings_of_the_worker_come_from_the_flag_the_variable_the_host_and_the_default() {
        let host = NodeConfig {
            node_id: NodeId::new("host").unwrap(),
            concurrency: 4,
            poll_interval: Duration::from_millis(700),
            default_max_turn: Duration::from_secs(60),
            shutdown_grace_period: Duration::from_secs(9),
            shutdown_cancel_timeout: Duration::from_secs(2),
            ..NodeConfig::default()
        };
        let from_host = http(&[], &Env::new(), Some(&host)).unwrap().worker;
        assert_eq!(from_host.worker_id.as_str(), "host");
        assert_eq!(from_host.concurrency, 4);
        assert_eq!(from_host.poll_interval, Duration::from_millis(700));
        assert_eq!(from_host.default_max_turn, Duration::from_secs(60));
        assert_eq!(from_host.shutdown_grace_period, Duration::from_secs(9));
        assert_eq!(from_host.shutdown_cancel_timeout, Duration::from_secs(2), "the host's cancel timeout is kept");

        let e = env(&[
            ("CICADA_NODE_ID", "env"),
            ("CICADA_CONCURRENCY", "8"),
            ("CICADA_POLL_INTERVAL", "100ms"),
            ("CICADA_MAX_TURN", "2m"),
            ("CICADA_SHUTDOWN_GRACE_PERIOD", "3s"),
        ]);
        let from_env = http(&[], &e, Some(&host)).unwrap().worker;
        assert_eq!(from_env.worker_id.as_str(), "env");
        assert_eq!(from_env.concurrency, 8);
        assert_eq!(from_env.poll_interval, Duration::from_millis(100));
        assert_eq!(from_env.default_max_turn, Duration::from_secs(120));
        assert_eq!(from_env.shutdown_grace_period, Duration::from_secs(3));

        let flags = [
            "--node-id",
            "flag",
            "--concurrency",
            "12",
            "--poll-interval",
            "50ms",
            "--max-turn",
            "1h",
            "--shutdown-grace-period",
            "1s",
        ];
        let from_flags = http(&flags, &e, Some(&host)).unwrap().worker;
        assert_eq!(from_flags.worker_id.as_str(), "flag");
        assert_eq!(from_flags.concurrency, 12);
        assert_eq!(from_flags.poll_interval, Duration::from_millis(50));
        assert_eq!(from_flags.default_max_turn, Duration::from_secs(3600));
        assert_eq!(from_flags.shutdown_grace_period, Duration::from_secs(1));

        let defaults = http(&[], &Env::new(), None).unwrap().worker;
        let library = WorkerConfig::default();
        assert_eq!(
            (defaults.concurrency, defaults.poll_interval, defaults.default_max_turn),
            (library.concurrency, library.poll_interval, library.default_max_turn)
        );
        assert_eq!(
            (defaults.shutdown_grace_period, defaults.shutdown_cancel_timeout),
            (library.shutdown_grace_period, library.shutdown_cancel_timeout)
        );
    }

    /// Scenarios: Label flag in HTTP mode; Unused label cannot break HTTP execution.
    #[test]
    fn a_worker_over_http_refuses_the_label_flag_and_never_reads_the_variable_or_the_host_s_label() {
        let error = http(&["--node-label", "x"], &Env::new(), None).unwrap_err();
        assert_eq!(error, "a worker over HTTP does not take --node-label: it has no row in the node table");
        let host = NodeConfig { label: Some(String::new()), concurrency: 3, ..NodeConfig::default() };
        let e = env(&[("CICADA_NODE_LABEL", " \t "), ("CICADA_CONCURRENCY", "7")]);
        assert_eq!(http(&[], &e, Some(&host)).unwrap().worker.concurrency, 7);
        assert_eq!(http(&[], &env(&[("CICADA_NODE_LABEL", "a\0b")]), Some(&host)).unwrap().worker.concurrency, 3);
    }

    /// Scenario: Applicable settings (the types), and a value that cannot be read.
    #[test]
    fn the_types_are_picked_as_on_the_store_and_a_bad_applicable_value_is_named() {
        assert_eq!(http(&["--types", "b"], &Env::new(), None).unwrap().types, ["b"]);
        assert_eq!(http(&[], &env(&[("CICADA_TYPES", "b,a")]), None).unwrap().types, ["a", "b"]);
        assert_eq!(http(&[], &Env::new(), None).unwrap().types, ["a", "b"]);
        let none = http(&["--types", "none"], &Env::new(), None).unwrap_err();
        assert!(none.contains("a worker must host a type"), "{none}");
        let error = http(&[], &env(&[("CICADA_CONCURRENCY", "many")]), None).unwrap_err();
        assert!(error.starts_with("CICADA_CONCURRENCY: invalid value 'many'"), "{error}");
    }

    /// Scenario: Explicit server settings.
    #[test]
    fn the_flags_of_settings_the_server_owns_are_refused_by_name() {
        for (flag, value) in [
            ("--store-schema", "orders"),
            ("--store-pool-size", "9"),
            ("--retention", "1d"),
            ("--lease-ttl", "5s"),
            ("--heartbeat-interval", "1s"),
            ("--max-lost-claims", "off"),
        ] {
            let error = http(&[flag, value], &Env::new(), None).unwrap_err();
            assert!(error.starts_with(&format!("a worker over HTTP does not take {flag}:")), "{error}");
            assert!(error.contains("the server owns"), "{error}");
        }
        let error = http(&["--lease-ttl", "5s", "--retention", "1d"], &Env::new(), None).unwrap_err();
        assert!(error.contains("--retention, --lease-ttl"), "every refused flag is named: {error}");
    }

    /// Scenario: Ignored settings cannot break HTTP execution.
    #[test]
    fn the_variables_and_host_values_the_server_owns_are_not_read() {
        let poisoned = env(&[
            ("CICADA_STORE_SCHEMA", "not;a;schema"),
            ("CICADA_STORE_POOL_SIZE", "lots"),
            ("CICADA_RETENTION", "forever"),
            ("CICADA_LEASE_TTL", "soon"),
            ("CICADA_HEARTBEAT_INTERVAL", "often"),
            ("CICADA_MAX_LOST_CLAIMS", "never"),
            ("CICADA_LISTEN", "localhost"),
        ]);
        let host = NodeConfig {
            lease_ttl: Duration::ZERO,
            heartbeat_interval: Some(Duration::ZERO),
            sweep_interval: Duration::ZERO,
            max_lost_claims: Some(0),
            ..NodeConfig::default()
        };
        let settings = http(&[], &poisoned, Some(&host)).expect("nothing of the server's is read");
        assert_eq!(settings.types, ["a", "b"]);
    }

    fn config_file(dir: &std::path::Path, text: &str) -> Env {
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        env(&[("CICADA_CONFIG", path.to_str().unwrap())])
    }

    /// Scenario: HTTP target resolution.
    #[test]
    fn the_target_is_resolved_as_for_the_operator_commands() {
        let dir = tempfile::tempdir().unwrap();
        let file = r#"
            current-context = "a"
            [contexts.a]
            server = "http://a"
            token = "ta"
            headers = { "X-Tenant" = "ops" }
            [contexts.b]
            server = "http://b"
        "#;
        let mut e = config_file(dir.path(), file);
        let target = http_target(&worker_args(&["--server", "http://flag"]), &e, "myapp").unwrap();
        assert_eq!(target.server, "http://flag");
        assert_eq!(target.token.as_deref(), Some("ta"), "the current context gives its token under a --server");
        assert_eq!(target.headers["X-Tenant"], "ops");
        e.insert("CICADA_SERVER".into(), "   ".into());
        e.insert("CICADA_CONTEXT".into(), "b".into());
        let target = http_target(&worker_args(&[]), &e, "myapp").unwrap();
        assert_eq!((target.server.as_str(), target.token.as_deref()), ("http://b", None), "a blank server is unset");
        e.insert("CICADA_CONTEXT".into(), " ".into());
        let target = http_target(&worker_args(&[]), &e, "myapp").unwrap();
        assert_eq!(target.server, "http://a", "a blank context is unset");
        let target = http_target(&worker_args(&["--context", "b", "--token", "tf"]), &e, "myapp").unwrap();
        assert_eq!((target.server.as_str(), target.token.as_deref()), ("http://b", Some("tf")));
        let error = http_target(&worker_args(&["--context", "nope"]), &e, "myapp").unwrap_err();
        assert!(error.to_string().contains("no context 'nope'"), "{error}");
        let broken = config_file(dir.path(), "this is = not [toml");
        let error = http_target(&worker_args(&["--server", "http://x"]), &broken, "myapp").unwrap_err();
        assert!(error.to_string().starts_with("cannot read"), "{error}");
    }

    /// The `Debug` of the settings of a worker over HTTP holds no credential: they are in the target, apart.
    #[test]
    fn the_debug_of_the_http_settings_holds_no_credential() {
        let settings = http(&[], &Env::new(), None).unwrap();
        let text = format!("{settings:?}");
        assert!(text.contains("worker_id") && !text.contains("token"), "{text}");
    }
}
