//! The wake-up listener: one dedicated connection that `LISTEN`s on the store's channel.

use std::sync::Mutex;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgListener, PgPoolOptions};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

/// Longest back-off between reconnects.
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
/// How often the listener sends itself a probe notification.
const PROBE_INTERVAL: Duration = Duration::from_secs(5);
/// How long the listener may receive nothing, not even its own probe, before it counts as hanging.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Payload of probe notifications. Never a template (templates contain `://`), so every listener ignores it, also
/// those of Kotlin nodes on the same channel.
pub(crate) const PROBE_PAYLOAD: &str = "cicada-listener-probe";

/// Holds one dedicated connection that `LISTEN`s on `channel` and republishes every notification's payload. Started
/// lazily by [`Self::start`]; runs until [`Self::stop`].
///
/// Robustness: the listener sends itself a probe notification every 5 s (sqlx only notices a dead socket when it
/// reads from it, and a dead peer without a TCP reset never answers); when nothing arrives for 10 s, or the connection
/// fails, it is dropped and re-opened after a back-off that starts at the configured delay and doubles up to 30 s.
/// After every reconnect an empty payload is published, because notifications sent while disconnected are lost:
/// subscribers treat it as "something may have changed".
pub(crate) struct NotificationListener {
    channel: String,
    options: PgConnectOptions,
    connection_timeout: Duration,
    reconnect_delay: Duration,
    notifications: broadcast::Sender<String>,
    listening: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
    stopped: std::sync::atomic::AtomicBool,
}

impl NotificationListener {
    pub(crate) fn new(
        channel: String,
        options: PgConnectOptions,
        connection_timeout: Duration,
        reconnect_delay: Duration,
    ) -> Self {
        Self {
            channel,
            options,
            connection_timeout,
            reconnect_delay,
            notifications: broadcast::channel(64).0,
            listening: watch::channel(false).0,
            task: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Payloads of received notifications; an empty string means "anything may have changed".
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<String> {
        self.notifications.subscribe()
    }

    /// True while `LISTEN` is active on a live connection.
    pub(crate) fn is_listening(&self) -> bool {
        *self.listening.borrow()
    }

    /// Starts listening, once. Must be called inside a tokio runtime.
    pub(crate) fn start(&self) {
        let mut task = self.task.lock().unwrap_or_else(|e| e.into_inner());
        if task.is_some() || self.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let run = Run {
            channel: self.channel.clone(),
            options: self.options.clone(),
            connection_timeout: self.connection_timeout,
            reconnect_delay: self.reconnect_delay,
            notifications: self.notifications.clone(),
            listening: self.listening.clone(),
        };
        *task = Some(tokio::spawn(run.run()));
    }

    /// Stops listening and closes the connections.
    pub(crate) async fn stop(&self) {
        self.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        self.listening.send_replace(false);
    }
}

impl Drop for NotificationListener {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
    }
}

struct Run {
    channel: String,
    options: PgConnectOptions,
    connection_timeout: Duration,
    reconnect_delay: Duration,
    notifications: broadcast::Sender<String>,
    listening: watch::Sender<bool>,
}

impl Run {
    async fn run(self) {
        let mut backoff = self.reconnect_delay;
        let mut reconnecting = false;
        loop {
            match self.connect().await {
                Ok((mut listener, pool)) => {
                    self.listening.send_replace(true);
                    if reconnecting {
                        self.publish(String::new());
                    }
                    reconnecting = true;
                    backoff = self.reconnect_delay;
                    let probe = tokio::spawn(probe(pool, self.channel.clone()));
                    let error = self.receive(&mut listener).await;
                    probe.abort();
                    self.listening.send_replace(false);
                    tracing::warn!(
                        "Wake-up listener on '{}' lost its connection ({error}); reconnecting in {backoff:?}",
                        self.channel
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Wake-up listener on '{}' cannot connect ({e}); retrying in {backoff:?}",
                        self.channel
                    )
                }
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_RECONNECT_DELAY);
        }
    }

    /// A listener on a fresh pool of two connections: one for `LISTEN`, one for probes. Fresh, so that a hanging
    /// connection of an earlier attempt (whose clean-up may never finish) cannot block this one.
    async fn connect(&self) -> Result<(PgListener, PgPool), sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .min_connections(0)
            .acquire_timeout(self.connection_timeout)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_lazy_with(self.options.clone());
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(&self.channel).await?;
        Ok((listener, pool))
    }

    /// Republishes notifications until the connection fails; returns why.
    async fn receive(&self, listener: &mut PgListener) -> String {
        loop {
            match timeout(PROBE_INTERVAL + PROBE_TIMEOUT, listener.try_recv()).await {
                Ok(Ok(Some(notification))) => {
                    if notification.payload() != PROBE_PAYLOAD {
                        self.publish(notification.payload().to_owned());
                    }
                }
                // The connection was lost and sqlx re-established it (and its LISTEN): notifications may be missed.
                Ok(Ok(None)) => self.publish(String::new()),
                Ok(Err(e)) => return e.to_string(),
                Err(_) => {
                    return format!("no notification, not even a probe, for {:?}", PROBE_INTERVAL + PROBE_TIMEOUT);
                }
            }
        }
    }

    fn publish(&self, payload: String) {
        let _ = self.notifications.send(payload); // no subscribers: nobody to wake
    }
}

/// Sends a probe notification on `channel` every [`PROBE_INTERVAL`].
async fn probe(pool: PgPool, channel: String) {
    loop {
        sleep(PROBE_INTERVAL).await;
        let sent = sqlx::query("SELECT pg_notify($1, $2)").bind(&channel).bind(PROBE_PAYLOAD).execute(&pool).await;
        if let Err(e) = sent {
            tracing::debug!("Wake-up listener probe on '{channel}' failed: {e}");
        }
    }
}
