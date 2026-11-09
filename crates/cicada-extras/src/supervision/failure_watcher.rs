use std::fmt;
use std::time::Duration;

use cicada::address::ActorRef;
use cicada::contract::canonical_sha256;
use cicada::runtime::{CreateActorResult, SendResult};
use cicada::store::{MessageId, OutcomeRecord, OutcomeStatus, PageCursor};
use cicada::time::{self, Timestamp};
use cicada::{ActorAddress, ActorSystem, SendOptions};
use serde_json::Value;
use tokio::task::JoinHandle;

use super::dead_letters::{DeadLetter, DeadLetters, DeadLettersMsg, DeadLettersState};
/// Why a poll of the [`FailureWatcher`] failed.
pub type WatchError = Box<dyn std::error::Error + Send + Sync>;

const PAGE: usize = 100;

/// Copies failed messages from the failed outcomes (`GET /outcomes?status=failed`) into the [`DeadLetters`] actor.
///
/// It talks to the actors through [`ActorOperations`]: in the service, next to the node, through the node itself
/// (`FailureWatcher::new(system)`), or in a process of its own through the HTTP client (`FailureWatcher::new(client)`,
/// with the `client` feature). Every node may run it at the same time: each failure is forwarded with an id derived from the failed
/// message's id ([`Self::dead_letter_id`]), and the store deduplicates sends by id, so a failure reaches the
/// dead-letter actor once however many watchers see it.
///
/// ```
/// # use std::time::Duration;
/// # async fn run(system: cicada::ActorSystem) {
/// use cicada_extras::FailureWatcher;
///
/// let watcher = FailureWatcher::new(system).prefix("orders/").start(Duration::from_secs(10));
/// // … on shutdown:
/// watcher.abort();
/// # }
/// ```
pub struct FailureWatcher {
    system: ActorSystem,
    prefix: String,
    overlap: Duration,
    dead_letters: ActorAddress,
    since: Option<Timestamp>,
    created: bool,
}

impl FailureWatcher {
    /// The default look-back of each poll.
    pub const DEFAULT_OVERLAP: Duration = Duration::from_secs(60);

    /// A watcher of every failure reachable through `ops` (an `ActorSystem`, or an `ActorClient` with the `client`
    /// feature).
    pub fn new(system: ActorSystem) -> Self {
        Self {
            system,
            prefix: String::new(),
            overlap: Self::DEFAULT_OVERLAP,
            dead_letters: DeadLetters.address().expect("the dead-letter address is valid"),
            since: None,
            created: false,
        }
    }

    /// Only failures of instances under this address prefix, e.g. `orders/`; empty (the default) for all.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// How far each poll looks back before the previous one's newest failure; 1 minute by default. Commits from other
    /// nodes may become visible slightly out of time order; re-reading this window costs nothing, thanks to the dedup.
    pub fn overlap(mut self, overlap: Duration) -> Self {
        self.overlap = overlap;
        self
    }

    /// Reads the failures since the last poll and forwards the new ones. Returns how many failures were forwarded for
    /// the first time.
    pub async fn poll(&mut self) -> Result<usize, WatchError> {
        if !self.created {
            // Idempotent.
            match self.system.create(&DeadLetters, &DeadLettersState::default()).await? {
                CreateActorResult::Created | CreateActorResult::Exists(_) => self.created = true,
                other => return Err(format!("Create of {} refused: {other:?}", self.dead_letters).into()),
            }
        }
        let mut newest = self.since;
        let mut forwarded = 0;
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = self
                .system
                .outcomes(Some(OutcomeStatus::Failed), &self.prefix, self.since, cursor.as_ref(), PAGE)
                .await?;
            for failure in page.items {
                if failure.actor == self.dead_letters {
                    continue; // a failed Redrive is not itself a dead letter
                }
                let finished_at = failure.finished_at;
                let id = Self::dead_letter_id(&failure.message_id);
                let record = DeadLettersMsg::RecordFailure { letter: to_dead_letter(failure) };
                match self.system.send_with(&DeadLetters, record, SendOptions::new().id(id)).await? {
                    SendResult::Accepted(_) => forwarded += 1,
                    SendResult::NoSuchInstance => self.created = false, // deleted meanwhile: re-create next time
                    SendResult::Duplicate(_) => {}                      // already forwarded, by this or another watcher
                    other => return Err(format!("Send to {} refused: {other:?}", self.dead_letters).into()),
                }
                if newest.is_none_or(|n| finished_at > n) {
                    newest = Some(finished_at);
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        if let Some(newest) = newest
            && self.since.is_none_or(|since| newest > since)
        {
            let back = time::sub(newest, self.overlap);
            self.since = Some(self.since.map_or(back, |since| since.max(back)));
        }
        Ok(forwarded)
    }

    /// Polls every `interval` on a tokio task until the returned handle is aborted; errors are logged and the next
    /// poll tries again.
    pub fn start(mut self, interval: Duration) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if let Err(e) = self.poll().await {
                    tracing::warn!("Failure watcher poll failed; retrying in {interval:?}: {e}");
                }
                tokio::time::sleep(interval).await;
            }
        })
    }

    /// The id under which the failure of `failed` is recorded: the same on every node (a name-based UUID,
    /// version 3, of `dead-letter/<id>`).
    pub fn dead_letter_id(failed: &MessageId) -> MessageId {
        let hash = canonical_sha256(&Value::String(failed.to_string()));
        MessageId::new(format!("dead-letter-{}", hash.trim_start_matches("sha256:"))).expect("the id is not blank")
    }
}

impl fmt::Debug for FailureWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FailureWatcher").field("prefix", &self.prefix).field("since", &self.since).finish()
    }
}

fn to_dead_letter(failure: OutcomeRecord) -> DeadLetter {
    DeadLetter {
        failed_message_id: failure.message_id.to_string(),
        actor: failure.actor.to_string(),
        message_type: failure.envelope.message_type,
        payload: failure.envelope.payload,
        correlation_id: failure.envelope.correlation_id,
        error: failure.error,
        failed_at: failure.finished_at,
        redriven_as: None,
        redrive_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_letter_ids_are_stable() {
        let failed = MessageId::new("0f4c1e5a-1b8e-4a0e-9b1a-3c2d5e6f7a8b").unwrap();
        let id = FailureWatcher::dead_letter_id(&failed);
        assert_eq!(id, FailureWatcher::dead_letter_id(&failed));
        assert!(id.as_str().starts_with("dead-letter-") && id.as_str().len() == 12 + 64, "{id}");
        assert_ne!(id, FailureWatcher::dead_letter_id(&MessageId::new("other").unwrap()));
    }
}
