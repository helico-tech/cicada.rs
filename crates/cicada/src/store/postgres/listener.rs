//! The listener: one dedicated connection that `LISTEN`s on the store's channel and feeds the store's change hub.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgListener, PgPoolOptions};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use crate::change::ChangeHub;
#[cfg(test)]
use crate::change::SourceState;
use crate::store::postgres::store::CONNECTION_TIMEOUT;

/// First delay before the listener reconnects after losing its connection; doubles up to [`MAX_RECONNECT_DELAY`].
const RECONNECT_DELAY: Duration = Duration::from_millis(250);
/// Longest back-off between reconnects.
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
/// How often the listener sends itself a probe notification.
const PROBE_INTERVAL: Duration = Duration::from_secs(5);
/// How long the listener may receive nothing, not even its own probe, before it counts as hanging.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Payload of probe notifications. Never a type name (the `!` is not allowed in one), so every listener ignores it.
pub(crate) const PROBE_PAYLOAD: &str = "!cicada-listener-probe";

/// Holds one dedicated connection that `LISTEN`s on `channel` and feeds the store's [`ChangeHub`]: the hub goes up only
/// once `LISTEN` succeeded, down when the connection is lost, and gets every notification's payload (a feed batch, or a
/// type name for the wake-ups). Started lazily by [`Self::start`]; runs until [`Self::request_stop`].
///
/// Robustness: the listener sends itself a probe notification every 5 s (sqlx only notices a dead socket when it
/// reads from it, and a dead peer without a TCP reset never answers); when nothing arrives for 10 s, or the connection
/// fails or is lost, it is dropped and re-opened after a back-off that starts at 250 ms and doubles up to 30 s. sqlx's
/// own eager reconnect is off, so that every reconnect goes through `LISTEN` before the hub goes up again:
/// notifications sent while disconnected are lost, and the hub's `Up` makes every subscriber resync and every wake-up
/// stream wake. The probe runs inside the listener's task, not in one of its own: it ends with its connection attempt,
/// or with the task when [`Self::request_stop`] or `Drop` aborts it, so nothing keeps the attempt's pool after the store closes.
pub(crate) struct NotificationListener {
    channel: String,
    options: PgConnectOptions,
    hub: Arc<ChangeHub>,
    /// The running task, and whether the listener was stopped for good: one lock, so a start cannot race a stop.
    task: Mutex<(Option<JoinHandle<()>>, bool)>,
}

impl NotificationListener {
    pub(crate) fn new(channel: String, options: PgConnectOptions, hub: Arc<ChangeHub>) -> Self {
        Self { channel, options, hub, task: Mutex::new((None, false)) }
    }

    /// True while `LISTEN` is active on a live connection.
    #[cfg(test)]
    pub(crate) fn is_listening(&self) -> bool {
        self.hub.state() == SourceState::Up
    }

    /// Starts listening, once, unless stopped. Must be called inside a tokio runtime.
    pub(crate) fn start(&self) {
        let mut task = self.task.lock().unwrap_or_else(|e| e.into_inner());
        if task.0.is_some() || task.1 {
            return;
        }
        let run = Run { channel: self.channel.clone(), options: self.options.clone(), hub: self.hub.clone() };
        task.0 = Some(tokio::spawn(run.run()));
    }

    /// Stops listening for good, without waiting: a later [`Self::start`] does nothing, and the running task is
    /// aborted but kept, for [`Self::take_task`]. The abort holds even when nobody joins the task.
    pub(crate) fn request_stop(&self) {
        let task = self.stopped();
        if let Some(running) = &task.0 {
            running.abort();
        }
    }

    /// Stops listening for good as [`Self::request_stop`] does, and hands over the aborted task for joining.
    pub(crate) fn take_task(&self) -> Option<JoinHandle<()>> {
        let task = self.stopped().0.take();
        if let Some(running) = &task {
            running.abort();
        }
        task
    }

    /// The task slot, marked stopped for good.
    fn stopped(&self) -> std::sync::MutexGuard<'_, (Option<JoinHandle<()>>, bool)> {
        let mut task = self.task.lock().unwrap_or_else(|e| e.into_inner());
        task.1 = true;
        task
    }

    /// Puts `task` in place of the listener's task, for the tests of close.
    #[cfg(test)]
    pub(crate) fn replace_task(&self, task: JoinHandle<()>) {
        if let Some(old) = self.task.lock().unwrap_or_else(|e| e.into_inner()).0.replace(task) {
            old.abort();
        }
    }
}

impl Drop for NotificationListener {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).0.take() {
            task.abort();
        }
    }
}

struct Run {
    channel: String,
    options: PgConnectOptions,
    hub: Arc<ChangeHub>,
}

impl Run {
    async fn run(self) {
        let mut backoff = RECONNECT_DELAY;
        loop {
            match self.connect().await {
                Ok((mut listener, pool)) => {
                    self.hub.up();
                    backoff = RECONNECT_DELAY;
                    // The probe runs inside this future, so it ends with the attempt, or with the task when it is
                    // aborted: no separate task can keep the attempt's pool alive.
                    let error = tokio::select! {
                        error = self.receive(&mut listener) => error,
                        never = probe(&pool, &self.channel) => match never {},
                    };
                    self.hub.down();
                    tracing::warn!(
                        "Listener on '{}' lost its connection ({error}); reconnecting in {backoff:?}",
                        self.channel
                    );
                }
                Err(e) => {
                    tracing::warn!("Listener on '{}' cannot connect ({e}); retrying in {backoff:?}", self.channel)
                }
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_RECONNECT_DELAY);
        }
    }

    /// A listener on a fresh pool of two connections: one for `LISTEN`, one for probes. Fresh, so that a hanging
    /// connection of an earlier attempt (whose clean-up may never finish) cannot block this one. Returns once `LISTEN`
    /// succeeded. The attempt's future owns the pool: dropping it releases Cicada's handles, while sqlx may still finish
    /// returning or closing a connection in its own tasks, which the listener does not wait for.
    async fn connect(&self) -> Result<(PgListener, PgPool), sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .min_connections(0)
            .acquire_timeout(CONNECTION_TIMEOUT)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_lazy_with(self.options.clone());
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.eager_reconnect(false);
        listener.listen(&self.channel).await?;
        Ok((listener, pool))
    }

    /// Feeds the hub with notifications until the connection fails or is lost; returns why.
    async fn receive(&self, listener: &mut PgListener) -> String {
        loop {
            match timeout(PROBE_INTERVAL + PROBE_TIMEOUT, listener.try_recv()).await {
                Ok(Ok(Some(notification))) => {
                    if notification.payload() != PROBE_PAYLOAD {
                        self.hub.payload(notification.payload());
                    }
                }
                Ok(Ok(None)) => return "the connection was lost".to_owned(),
                Ok(Err(e)) => return e.to_string(),
                Err(_) => {
                    return format!("no notification, not even a probe, for {:?}", PROBE_INTERVAL + PROBE_TIMEOUT);
                }
            }
        }
    }
}

/// Sends a probe notification on `channel` every [`PROBE_INTERVAL`], on the attempt's `pool`; never returns, so it ends
/// only when the attempt's future drops it.
async fn probe(pool: &PgPool, channel: &str) -> Infallible {
    loop {
        sleep(PROBE_INTERVAL).await;
        let sent = sqlx::query("SELECT pg_notify($1, $2)").bind(channel).bind(PROBE_PAYLOAD).execute(pool).await;
        if let Err(e) = sent {
            tracing::debug!("Listener probe on '{channel}' failed: {e}");
        }
    }
}
