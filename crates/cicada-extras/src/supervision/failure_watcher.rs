use std::time::Duration;

use cicada::address::ActorRef;
use cicada::runtime::{OpError, SendResult};
use cicada::store::{MessageId, OutcomeRecord, Page, PageCursor};
use cicada::time::{self, Timestamp};
use cicada::{ActorAddress, ActorSystem, SendOptions};
use md5::{Digest, Md5};
use tokio::task::JoinHandle;

use super::dead_letters::{DeadLetter, DeadLetters, DeadLettersMsg, DeadLettersState};

/// Copies failed messages from the failures list (`GET /failures`) into the [`DeadLetters`] actor.
///
/// It runs in the service, next to the node, and talks to the actors through the node's operations (the same as the
/// HTTP API). Every node may run it at the same time: each failure is forwarded with an id derived from the failed
/// message's id ([`Self::dead_letter_id`]), and the store deduplicates sends by id, so a failure reaches the
/// dead-letter actor once however many watchers see it.
///
/// ```
/// # use std::time::Duration;
/// # async fn run(system: cicada::ActorSystem) {
/// use cicada_extras::FailureWatcher;
///
/// let watcher = FailureWatcher::new(system).prefix("actor://orders/").start(Duration::from_secs(10));
/// // … on shutdown:
/// watcher.abort();
/// # }
/// ```
#[derive(Debug)]
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

    /// A watcher of every failure on `system`'s store.
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

    /// Only failures of instances under this URI prefix, e.g. `actor://orders/`; empty (the default) for all.
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
    pub async fn poll(&mut self) -> Result<usize, OpError> {
        if !self.created {
            self.system.create(&DeadLetters, &DeadLettersState::default()).await?; // idempotent
            self.created = true;
        }
        let mut newest = self.since;
        let mut forwarded = 0;
        let mut cursor: Option<PageCursor> = None;
        loop {
            let page = self.system.failures(&self.prefix, self.since, cursor.as_ref(), Page::<()>::DEFAULT_SIZE).await?;
            for failure in page.items {
                if failure.actor == self.dead_letters {
                    continue; // a failed Redrive is not itself a dead letter
                }
                let finished_at = failure.finished_at;
                let options = SendOptions::new().id(Self::dead_letter_id(&failure.message_id));
                let record = DeadLettersMsg::RecordFailure { letter: to_dead_letter(failure) };
                match self.system.send_with(&DeadLetters, record, options).await? {
                    SendResult::Accepted(_) => forwarded += 1,
                    SendResult::NoSuchInstance => self.created = false, // deleted meanwhile: re-create next time
                    _ => {} // a duplicate: already forwarded, by this or another watcher
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

    /// The id under which the failure of `failed` is recorded: the same on every node, and the same as the Kotlin
    /// implementation's (a name-based UUID, version 3, of `dead-letter/<id>`).
    pub fn dead_letter_id(failed: &MessageId) -> MessageId {
        let digest: [u8; 16] = Md5::digest(format!("dead-letter/{failed}").as_bytes()).into();
        let uuid = uuid::Builder::from_md5_bytes(digest).into_uuid();
        MessageId::new(uuid.to_string()).expect("a UUID is not blank")
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
    fn dead_letter_ids_match_the_kotlin_implementation() {
        // UUID.nameUUIDFromBytes("dead-letter/0f4c1e5a-1b8e-4a0e-9b1a-3c2d5e6f7a8b".toByteArray())
        let failed = MessageId::new("0f4c1e5a-1b8e-4a0e-9b1a-3c2d5e6f7a8b").unwrap();
        assert_eq!(FailureWatcher::dead_letter_id(&failed).as_str(), "75de82e8-12d3-38e1-955b-632295d120fb");
    }
}
