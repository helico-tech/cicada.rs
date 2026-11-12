//! The settings of `serve` and `worker`: the store, the address, the types and the node. Each comes from its flag, else
//! its variable `CICADA_…` (a blank one counts as unset), else for the node the host's `NodeConfig`, else the default.
//! [`resolve`] is a function of the flags, the variables and the host's values, so it is tested without a process.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use cicada::runtime::NodeConfig;
use cicada::store::NodeId;
use clap::{Args, Parser};

use crate::Env;

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
}

/// The flags that `serve` and `worker` share.
#[derive(Debug, Clone, Default, Args)]
pub(crate) struct NodeArgs {
    /// Which store: a postgres:// or postgresql:// address, sqlite:FILE, or the path of a SQLite file
    /// [env: CICADA_STORE]
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
pub(crate) struct Settings {
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
    /// The node.
    pub node: NodeConfig,
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

/// The settings of `kind` from its flags `args`, `listen` (`serve` only), the variables `env`, the host's `NodeConfig`
/// and the names of the types the binary hosts. An error is a command line that cannot be used: it names the flag or
/// the variable.
pub(crate) fn resolve(
    kind: Kind,
    args: &NodeArgs,
    listen: Option<SocketAddr>,
    env: &Env,
    host: Option<&NodeConfig>,
    hosted: &[&'static str],
) -> Result<Settings, String> {
    let store = match (&args.store, variable(env, "CICADA_STORE")) {
        (Some(value), _) => StoreSpec::parse(value).map_err(|e| format!("--store: {e}"))?,
        (None, Some(value)) => StoreSpec::parse(value).map_err(|e| format!("CICADA_STORE: {e}"))?,
        (None, None) => {
            return Err(format!("{} needs a store: pass --store or set CICADA_STORE", kind.name()));
        }
    };
    let types = match (&args.types, variable(env, "CICADA_TYPES")) {
        (Some(value), _) => pick_types(kind, value, hosted).map_err(|e| format!("--types: {e}"))?,
        (None, Some(value)) => pick_types(kind, value, hosted).map_err(|e| format!("CICADA_TYPES: {e}"))?,
        (None, None) => hosted.to_vec(),
    };

    let mut node = host.cloned().unwrap_or_default();
    if let Some(id) = setting(args.node_id.clone(), env, "CICADA_NODE_ID", node_id)? {
        node.node_id = id;
    }
    if let Some(concurrency) = setting(args.concurrency, env, "CICADA_CONCURRENCY", number)? {
        node.concurrency = concurrency;
    }
    if let Some(ttl) = setting(args.lease_ttl, env, "CICADA_LEASE_TTL", duration)? {
        node.lease_ttl = ttl;
    }
    if let Some(heartbeat) = setting(args.heartbeat_interval, env, "CICADA_HEARTBEAT_INTERVAL", duration)? {
        node.heartbeat_interval = Some(heartbeat);
    }
    if let Some(poll) = setting(args.poll_interval, env, "CICADA_POLL_INTERVAL", duration)? {
        node.poll_interval = poll;
    }
    if let Some(grace) = setting(args.shutdown_grace_period, env, "CICADA_SHUTDOWN_GRACE_PERIOD", duration)? {
        node.shutdown_grace_period = grace;
    }
    if let Some(max_turn) = setting(args.max_turn, env, "CICADA_MAX_TURN", duration)? {
        node.default_max_turn = max_turn;
    }
    if let Some(LostClaims(limit)) = setting(args.max_lost_claims, env, "CICADA_MAX_LOST_CLAIMS", lost_claims)? {
        node.max_lost_claims = limit;
    }

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
        return Err(format!("an empty name in '{value}'"));
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
        resolve(kind, &args, listen, env, host, hosted)
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
        assert!(error.unwrap_err().contains("an empty name"));
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
}
