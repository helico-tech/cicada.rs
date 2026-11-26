//! The member of a placement pool: a node's stable identity in a file, and the loop that offers the node's places
//! while the host's probe answers.

use std::fs;
use std::future::Future;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::pin::pin;
use std::time::Duration;

use cicada::prelude::*;
use cicada::runtime::SendResult;
use cicada::store::NodeId;
use serde::{Deserialize, Serialize};
use tokio::time::MissedTickBehavior;

use super::{InvalidPlacement, Placement, PlacementMsg, check_id};

/// The name of the identity file in the folder given to [`NodeIdentity::load_or_create`].
const IDENTITY_FILE: &str = "placement-identity.json";

/// A node's identity in a placement pool, kept in `placement-identity.json`: the node id, the default for
/// [`NodeConfig::node_id`](cicada::runtime::NodeConfig::node_id), and the instance, which goes into each offer and
/// stays across restarts, so a restarted node is the same machine to the pool. A copy of the file on another machine
/// makes it the same machine too: never copy it, or a disk that holds it, to another node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeIdentity {
    node_id: NodeId,
    instance: String,
}

/// The identity file's JSON.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    node: String,
    instance: String,
}

impl NodeIdentity {
    /// Reads the identity from `dir/placement-identity.json`, or creates `dir` when missing and the file with the node
    /// id `node-<16 hex digits>` and a random instance, written to a temporary file and renamed into place. A file that
    /// cannot be read, or is not `{"node": …, "instance": …}` with two ids of 1 through 200 characters, not blank,
    /// without NUL, is an [`InvalidIdentity`] and stays as it is: a node never takes a new identity by accident. One
    /// process creates the file; two that start at once on a missing file may each write one.
    ///
    /// ```
    /// use cicada_actors::NodeIdentity;
    ///
    /// let dir = tempfile::tempdir()?;
    /// let first = NodeIdentity::load_or_create(dir.path().join("placement"))?;
    /// assert!(first.node_id().as_str().starts_with("node-"));
    /// assert_eq!(NodeIdentity::load_or_create(dir.path().join("placement"))?, first); // after a restart
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load_or_create(dir: impl AsRef<Path>) -> Result<Self, InvalidIdentity> {
        let dir = dir.as_ref();
        let path = dir.join(IDENTITY_FILE);
        let fail = |what: &str, e: &dyn std::fmt::Display| InvalidIdentity(format!("{}: {what}: {e}", path.display()));
        match fs::read(&path) {
            Ok(bytes) => {
                let file: IdentityFile =
                    serde_json::from_slice(&bytes).map_err(|e| fail("not the identity JSON", &e))?;
                Self::checked(file.node, file.instance).map_err(|e| fail("invalid", &e))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let hex = uuid::Uuid::new_v4().simple().to_string();
                let identity = Self::checked(format!("node-{}", &hex[..16]), uuid::Uuid::new_v4().simple().to_string())
                    .map_err(|e| fail("invalid", &e))?;
                identity.write(dir).map_err(|e| fail("cannot be written", &e))?;
                Ok(identity)
            }
            Err(e) => Err(fail("cannot be read", &e)),
        }
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
        Ok(Self { node_id, instance })
    }

    /// Writes the file atomically: a temporary file in `dir`, synced, then renamed into place.
    fn write(&self, dir: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
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

/// An identity file that cannot be read, written or parsed; it prints the path and why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("placement identity {0}")]
pub struct InvalidIdentity(String);

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

/// What a node can take now, as the probe of a [`PlacementMember`] gives it: the labels and places of its offer, in
/// the bounds of [`PlacementMsg::offer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capacity {
    /// What the node offers, such as its processor kind; sorted and kept once in the offer.
    pub labels: Vec<String>,
    /// How many places the node offers.
    pub places: u32,
}

impl Capacity {
    /// `places` places with `labels`.
    pub fn new<L: Into<String>>(labels: impl IntoIterator<Item = L>, places: u32) -> Self {
        Self { labels: labels.into_iter().map(Into::into).collect(), places }
    }
}

/// Offers a node's places to a [`Placement`] pool while the node can run work. It runs as a task, not in actor turns:
/// a host runs [`run`](Self::run) as an app task, once per node and pool.
///
/// **Rounds.** A round runs at the start of `run` and then every [`interval`](MemberSettings::interval); a round that
/// takes longer delays the next one. It calls the probe: a [`Capacity`] sends
/// `Offer { node, instance, labels, places }` to the pool, with the node's own id ([`ActorSystem::node_id`], the one its
/// claims and pins use, also when `--node-id` overrides the identity's) and the identity's instance; `None` sends
/// nothing, so a node whose runtime does not answer goes silent at the pool by itself.
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
    instance: String,
    settings: MemberSettings,
    probe: P,
}

impl<P> std::fmt::Debug for PlacementMember<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlacementMember")
            .field("pool", &self.pool.pool)
            .field("instance", &self.instance)
            .field("settings", &self.settings)
            .finish()
    }
}

impl<P, F> PlacementMember<P>
where
    P: FnMut() -> F,
    F: Future<Output = Option<Capacity>>,
{
    /// A member that offers to `pool` under `identity`'s instance what `probe` gives, each round.
    pub fn new(pool: Placement, identity: NodeIdentity, settings: MemberSettings, probe: P) -> Self {
        Self { pool, instance: identity.instance, settings, probe }
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

    /// Calls the probe and sends its offer.
    async fn round(&mut self, system: &ActorSystem) {
        let Some(Capacity { labels, places }) = (self.probe)().await else {
            return;
        };
        let pool = self.pool.pool.as_str();
        let offer = match PlacementMsg::offer(system.node_id().as_str(), &self.instance, labels, places) {
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
