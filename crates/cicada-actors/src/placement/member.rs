//! The member of a placement pool: a node's stable identity in a file, and the loop that offers the node's places
//! while the host's probe answers.

use std::fs::{self, TryLockError};
use std::future::Future;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::SendResult;
use cicada::store::NodeId;
use serde::{Deserialize, Serialize};
use tokio::time::MissedTickBehavior;

use super::{InvalidPlacement, Placement, PlacementMsg, check_id};

/// The name of the identity file in the folder given to [`NodeIdentity::load_or_create`].
const IDENTITY_FILE: &str = "placement-identity.json";
/// The name of the file locked while an identity of the folder lives.
const LOCK_FILE: &str = "placement-identity.lock";

/// A node's identity in a placement pool, kept in `placement-identity.json`: the node id, the default for
/// [`NodeConfig::node_id`](cicada::runtime::NodeConfig::node_id), and the instance, which goes into each offer and
/// stays across restarts, so a restarted node is the same machine to the pool. A copy of the file on another machine
/// makes it the same machine too: never copy it, or a disk that holds it, to another node.
///
/// **The lock.** While the identity or a clone of it lives, it holds an exclusive OS lock on
/// `placement-identity.lock` in its folder, released when the last clone is dropped or the process ends, so a second
/// process on the same folder fails to load it with [`IdentityErrorKind::Locked`]. Give the identity to the
/// [`PlacementMember`], which keeps it while it runs. The lock covers one machine only: a copied folder on another
/// machine is caught by the member's duplicate check, and only when the two machines' host names differ. On one host
/// with a shared volume, a rolling deploy's new process fails with `Locked` until the old one stops. On a file system
/// without locks, and in a folder where the lock file cannot be created or opened for writing for lack of permission
/// (a read-only folder that holds the identity file), it logs a warning and goes on without one. Equality compares
/// the node id and the instance.
#[derive(Clone)]
pub struct NodeIdentity {
    node_id: NodeId,
    instance: String,
    /// The locked file; none on a file system without locks.
    lock: Option<Arc<fs::File>>,
}

impl PartialEq for NodeIdentity {
    fn eq(&self, other: &Self) -> bool {
        (&self.node_id, &self.instance) == (&other.node_id, &other.instance)
    }
}

impl Eq for NodeIdentity {}

impl std::fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeIdentity")
            .field("node_id", &self.node_id)
            .field("instance", &self.instance)
            .field("locked", &self.lock.is_some())
            .finish()
    }
}

/// The identity file's JSON.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    node: String,
    instance: String,
}

impl NodeIdentity {
    /// Locks the folder `dir`, creating it when missing, then reads the identity from `dir/placement-identity.json`,
    /// or creates the file with the node id `node-<16 hex digits>` and a random instance, written to a temporary file
    /// and renamed into place. A folder locked by another live identity fails with [`IdentityErrorKind::Locked`]. A
    /// file that cannot be read, or is not `{"node": …, "instance": …}` with two ids of 1 through 200 characters, not
    /// blank, without NUL, is an [`InvalidIdentity`] of the kind [`IdentityErrorKind::Invalid`] and stays as it is: a
    /// node never takes a new identity by accident.
    ///
    /// ```
    /// use cicada_actors::{IdentityErrorKind, NodeIdentity};
    ///
    /// let dir = tempfile::tempdir()?;
    /// let first = NodeIdentity::load_or_create(dir.path().join("placement"))?;
    /// assert!(first.node_id().as_str().starts_with("node-"));
    /// let second = NodeIdentity::load_or_create(dir.path().join("placement")).unwrap_err();
    /// assert_eq!(second.kind(), IdentityErrorKind::Locked); // a second process on the folder
    /// let (node, instance) = (first.node_id(), first.instance().to_owned());
    /// drop(first);
    /// let restarted = NodeIdentity::load_or_create(dir.path().join("placement"))?; // after a restart
    /// assert_eq!((restarted.node_id(), restarted.instance()), (node, instance.as_str()));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load_or_create(dir: impl AsRef<Path>) -> Result<Self, InvalidIdentity> {
        let dir = dir.as_ref();
        let lock = lock(dir)?;
        let path = dir.join(IDENTITY_FILE);
        let fail = |what: &str, e: &dyn std::fmt::Display| {
            InvalidIdentity::invalid(format!("{}: {what}: {e}", path.display()))
        };
        let mut identity = match fs::read(&path) {
            Ok(bytes) => {
                let file: IdentityFile =
                    serde_json::from_slice(&bytes).map_err(|e| fail("not the identity JSON", &e))?;
                Self::checked(file.node, file.instance).map_err(|e| fail("invalid", &e))?
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let hex = uuid::Uuid::new_v4().simple().to_string();
                let identity = Self::checked(format!("node-{}", &hex[..16]), uuid::Uuid::new_v4().simple().to_string())
                    .map_err(|e| fail("invalid", &e))?;
                identity.write(dir).map_err(|e| fail("cannot be written", &e))?;
                identity
            }
            Err(e) => return Err(fail("cannot be read", &e)),
        };
        identity.lock = lock;
        Ok(identity)
    }

    /// The node id: the default for [`NodeConfig::node_id`](cicada::runtime::NodeConfig::node_id).
    pub fn node_id(&self) -> NodeId {
        self.node_id.clone()
    }

    /// The instance, which the member puts into each offer.
    pub fn instance(&self) -> &str {
        &self.instance
    }

    fn checked(node: String, instance: String) -> Result<Self, InvalidPlacement> {
        check_id("a node", &node)?;
        check_id("an instance", &instance)?;
        let node_id = NodeId::new(node).map_err(|e| InvalidPlacement(format!("a node id: {e}")))?;
        Ok(Self { node_id, instance, lock: None })
    }

    /// Writes the file atomically: a temporary file in `dir`, synced, then renamed into place.
    fn write(&self, dir: &Path) -> std::io::Result<()> {
        let file = IdentityFile { node: self.node_id.as_str().to_owned(), instance: self.instance.clone() };
        let temporary = dir.join(format!(".{IDENTITY_FILE}.{}", self.instance));
        let mut out = fs::File::create(&temporary)?;
        out.write_all(&serde_json::to_vec(&file).map_err(std::io::Error::other)?)?;
        out.sync_all()?;
        fs::rename(&temporary, dir.join(IDENTITY_FILE))?;
        #[cfg(unix)]
        fs::File::open(dir)?.sync_all()?;
        Ok(())
    }
}

/// Creates `dir` when missing and takes the exclusive lock on its lock file: `Locked` when another holds it, none with
/// a warning on a file system without locks or when the lock file cannot be opened for writing for lack of permission.
fn lock(dir: &Path) -> Result<Option<Arc<fs::File>>, InvalidIdentity> {
    let path = dir.join(LOCK_FILE);
    let fail =
        |what: &str, e: &dyn std::fmt::Display| InvalidIdentity::invalid(format!("{}: {what}: {e}", path.display()));
    fs::create_dir_all(dir).map_err(|e| fail("its folder cannot be created", &e))?;
    let file = match fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path) {
        Ok(file) => file,
        Err(e) if matches!(e.kind(), ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem) => {
            tracing::warn!("placement identity {}: no lock, as the lock file cannot be opened: {e}", path.display());
            return Ok(None);
        }
        Err(e) => return Err(fail("cannot be opened", &e)),
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(Arc::new(file))),
        Err(TryLockError::WouldBlock) => Err(InvalidIdentity {
            kind: IdentityErrorKind::Locked,
            message: format!("{}: locked by another process that runs this identity", path.display()),
        }),
        Err(TryLockError::Error(e)) => {
            tracing::warn!("placement identity {}: no lock, as the file system gave: {e}", path.display());
            Ok(None)
        }
    }
}

/// An identity that cannot be loaded: a file that cannot be read, written or parsed, or a folder that another process
/// holds; it prints the path and why, and [`kind`](Self::kind) tells which.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("placement identity {message}")]
pub struct InvalidIdentity {
    kind: IdentityErrorKind,
    message: String,
}

impl InvalidIdentity {
    fn invalid(message: String) -> Self {
        Self { kind: IdentityErrorKind::Invalid, message }
    }

    /// What went wrong.
    pub fn kind(&self) -> IdentityErrorKind {
        self.kind
    }
}

/// What went wrong with an [`InvalidIdentity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IdentityErrorKind {
    /// The folder or the file cannot be read, written or parsed, or holds ids out of bounds.
    Invalid,
    /// Another live identity, of this process or another on the machine, holds the folder's lock.
    Locked,
}

/// The settings of a [`PlacementMember`].
///
/// ```
/// use std::time::Duration;
/// use cicada_actors::MemberSettings;
///
/// assert_eq!(MemberSettings::new().interval(), Duration::from_secs(15));
/// let settings = MemberSettings::new().with_interval(Duration::from_secs(5))?;
/// assert!(MemberSettings::new().with_interval(Duration::ZERO).is_err());
/// # Ok::<(), cicada_actors::InvalidPlacement>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSettings {
    interval: Duration,
}

impl MemberSettings {
    /// The defaults: an interval of 15 seconds.
    pub fn new() -> Self {
        Self { interval: Duration::from_secs(15) }
    }

    /// These settings with the time between rounds `interval`; fails when it is zero. Keep it well below the pool's
    /// silence time, which the member cannot read, so that one lost offer does not make the node silent.
    pub fn with_interval(mut self, interval: Duration) -> Result<Self, InvalidPlacement> {
        if interval.is_zero() {
            return Err(InvalidPlacement("a member's interval is zero".into()));
        }
        self.interval = interval;
        Ok(self)
    }

    /// The time between rounds.
    pub fn interval(&self) -> Duration {
        self.interval
    }
}

impl Default for MemberSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// The host name in the node's row when it is set and differs from the one the node writes; none when either is
/// missing or the read fails.
async fn other_host(system: &ActorSystem, pool: &str) -> Option<String> {
    let own = system.hostname()?;
    match system.node(system.node_id()).await {
        Ok(row) => row.and_then(|row| row.hostname).filter(|host| host != own),
        Err(e) => {
            tracing::warn!(pool, "placement member: the node's row cannot be read, so no duplicate check: {e}");
            None
        }
    }
}

/// What a node can take now, as the probe of a [`PlacementMember`] gives it: the labels and places of its offer, in
/// the bounds of [`PlacementMsg::offer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capacity {
    /// What the node offers, such as its processor kind; sorted and kept once in the offer.
    pub labels: Vec<String>,
    /// How many places the node offers.
    pub places: u32,
    /// The resource whose places these are, such as the Docker daemon's id: the pool counts a key on one live node at
    /// a time (ADR 0072). None: the places are the node's own.
    pub resource: Option<String>,
}

impl Capacity {
    /// `places` places with `labels`, without a resource key.
    pub fn new<L: Into<String>>(labels: impl IntoIterator<Item = L>, places: u32) -> Self {
        Self { labels: labels.into_iter().map(Into::into).collect(), places, resource: None }
    }

    /// The same capacity counted against the resource `key`, such as the Docker daemon's id from `RuntimeInfo::id` of
    /// `cicada-containers`, so that two nodes that share one daemon do not offer its places twice: while one live node
    /// holds the key, the pool ignores the other's offers (ADR 0072). 1 through 200 characters without NUL, or the
    /// member sends no offer and warns.
    pub fn resource(self, key: impl Into<String>) -> Self {
        Self { resource: Some(key.into()), ..self }
    }
}

/// Offers a node's places to a [`Placement`] pool while the node can run work. It runs as a task, not in actor turns:
/// a host runs [`run`](Self::run) as an app task, once per node and pool.
///
/// **Rounds.** A round runs at the start of `run` and then every [`interval`](MemberSettings::interval); a round that
/// takes longer delays the next one. It calls the probe: a [`Capacity`] sends
/// `Offer { node, instance, labels, places, resource }` to the pool, with the node's own id ([`ActorSystem::node_id`], the one its
/// claims and pins use, also when `--node-id` overrides the identity's) and the identity's instance; `None` sends
/// nothing, so a node whose runtime does not answer goes silent at the pool by itself.
///
/// **Duplicates** (ADR 0071). Each round first reads the node's own row in the node table
/// ([`ActorSystem::node`]). Two machines under one node id write that one row in turn, so a host name there that is set
/// and differs from the one this node writes ([`ActorSystem::hostname`], see
/// [`NodeConfig::hostname`](cicada::runtime::NodeConfig::hostname)) means another machine runs under this id, such as
/// one started from a copied identity folder. That round then skips the probe and the offer, sends the pool
/// `Duplicate { node, host }`, and logs an error, once for each other host name in a row; a round that finds no other
/// host ends that run, so a later one is logged again. A round without a host name of
/// its own or in the row, or whose read fails (a warning), checks nothing. A redeploy under a new host name does not
/// trip it, as the old node removes its row when it stops; a node that crashed leaves its row for one more heartbeat of
/// the new one at most. Two machines that keep one host name are not told apart.
///
/// **Errors.** Nothing ends `run` but `stop`, also while a probe or a send hangs. An offer out of bounds and a send
/// that fails or is not accepted, such as to a pool not created yet, are logged as warnings and tried in the next
/// round.
///
/// ```
/// use std::time::Duration;
/// use cicada_actors::{
///     Capacity, MemberSettings, NodeIdentity, Placement, PlacementActor, PlacementMember, PlacementState,
/// };
/// use cicada_test::TestActors;
///
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dir = tempfile::tempdir()?;
/// let identity = NodeIdentity::load_or_create(dir.path())?;
/// let test = TestActors::builder().with(|node| node.host(PlacementActor)).build().await?;
/// let state = PlacementState::new(Duration::from_secs(60), 10, 10, 10)?;
/// test.create(&Placement::named("runners"), &state).await;
///
/// let member = PlacementMember::new(Placement::named("runners"), identity, MemberSettings::new(), || async {
///     Some(Capacity::new(["arm64", "docker"], 4)) // the host asks its runtime here, such as Docker's info()
/// });
/// member.run(test.system().clone(), tokio::time::sleep(Duration::from_secs(1))).await;
/// test.run_until_idle().await;
/// let pool: PlacementState = test.state(&Placement::named("runners")).await;
/// assert_eq!(pool.nodes[test.system().node_id().as_str()].places, 4);
/// # Ok(())
/// # }
/// ```
pub struct PlacementMember<P> {
    pool: Placement,
    /// Kept whole, so its lock lives as long as the member.
    identity: NodeIdentity,
    settings: MemberSettings,
    probe: P,
    /// The other host name last logged as a duplicate.
    logged: Option<String>,
}

impl<P> std::fmt::Debug for PlacementMember<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlacementMember")
            .field("pool", &self.pool.pool)
            .field("instance", &self.identity.instance)
            .field("settings", &self.settings)
            .finish()
    }
}

impl<P, F> PlacementMember<P>
where
    P: FnMut() -> F,
    F: Future<Output = Option<Capacity>>,
{
    /// A member that offers to `pool` under `identity`'s instance what `probe` gives, each round. It keeps `identity`,
    /// and so the lock of its folder, until it is dropped.
    pub fn new(pool: Placement, identity: NodeIdentity, settings: MemberSettings, probe: P) -> Self {
        Self { pool, identity, settings, probe, logged: None }
    }

    /// Offers in rounds through `system` until `stop` completes, then returns. No error ends it.
    pub async fn run(mut self, system: ActorSystem, stop: impl Future<Output = ()>) {
        let mut stop = pin!(stop);
        let mut rounds = tokio::time::interval(self.settings.interval);
        rounds.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = &mut stop => return,
                _ = rounds.tick() => {
                    tokio::select! {
                        () = &mut stop => return,
                        () = self.round(&system) => {}
                    }
                }
            }
        }
    }

    /// Reports another machine under the node's id, or calls the probe and sends its offer.
    async fn round(&mut self, system: &ActorSystem) {
        let pool = self.pool.pool.as_str();
        let Some(host) = other_host(system, pool).await else {
            // A later duplicate with the same host is a new one, logged again.
            self.logged = None;
            return self.offer(system).await;
        };
        let node = system.node_id().as_str();
        if self.logged.as_deref() != Some(host.as_str()) {
            let own = system.hostname().unwrap_or_default();
            tracing::error!(
                pool,
                node,
                host,
                "placement member: another machine, '{host}', runs under this node id ({node}) on this host \
                 ('{own}'): no offers while it does; give each machine an identity folder of its own"
            );
            self.logged = Some(host.clone());
        }
        let report = PlacementMsg::Duplicate { node: node.to_owned(), host };
        match system.send(&self.pool, report).await {
            Ok(SendResult::Accepted(_)) => {}
            Ok(other) => tracing::warn!(pool, "placement member: the duplicate report was not accepted: {other:?}"),
            Err(e) => tracing::warn!(pool, "placement member: the duplicate report failed: {e}"),
        }
    }

    /// Calls the probe and sends its offer.
    async fn offer(&mut self, system: &ActorSystem) {
        let pool = self.pool.pool.as_str();
        let Some(Capacity { labels, places, resource }) = (self.probe)().await else {
            return;
        };
        let (node, instance) = (system.node_id().as_str().to_owned(), self.identity.instance.clone());
        let offer = match PlacementMsg::offer_of(node, instance, labels, places, resource) {
            Ok(offer) => offer,
            Err(e) => {
                tracing::warn!(pool, "placement member: no offer: {e}");
                return;
            }
        };
        match system.send(&self.pool, offer).await {
            Ok(SendResult::Accepted(_)) => {}
            Ok(other) => tracing::warn!(pool, "placement member: the offer was not accepted: {other:?}"),
            Err(e) => tracing::warn!(pool, "placement member: the offer failed: {e}"),
        }
    }
}
