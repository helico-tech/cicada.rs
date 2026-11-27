//! Live updates against the real actor API: hints of `GET /changes` from a `TestActors` store, on the real clock.

use std::sync::Mutex;

use cicada_client::Change;
use futures::StreamExt;
use tokio::runtime::Handle;
use tokio::time::{Instant, sleep};

use super::*;
use crate::live::{ChangeSource, Hints, Live, LiveStatus, Pages};

type Received = Arc<Mutex<Vec<(Instant, Change)>>>;

/// The client's change stream, recording when each hint is received.
struct Tap {
    client: ActorClient,
    received: Received,
}

impl ChangeSource for Tap {
    fn open(&self, prefix: String) -> impl Future<Output = Result<Hints, String>> + Send + 'static {
        let (open, received) = (ChangeSource::open(&self.client, prefix), self.received.clone());
        async move {
            let hints = open.await?.inspect(move |hint| {
                if let Ok(change) = hint {
                    received.lock().unwrap().push((Instant::now(), change.clone()));
                }
            });
            Ok(Box::pin(hints) as Hints)
        }
    }
}

const ROOTS: &str = "served /actors/states?limit=100";

async fn served(server: &mut Server, event: &str) -> Instant {
    loop {
        let next = timeout(Duration::from_secs(10), server.events.recv()).await.expect("a request in time").unwrap();
        if next == event {
            return Instant::now();
        }
    }
}

/// Scenario: A hint for an actor, on an idle Actors page.
#[tokio::test]
async fn a_real_change_starts_the_actors_soft_refresh_within_a_second_of_its_hint() {
    let actors = actors().await;
    actors.create(&Counter { id: "a".into() }, &json!({})).await;
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(config(&dir, &[("local", &server.url)]), Handle::current(), Limits::DEFAULT));
    let mut nav = Navigation::default();
    session.use_context(None);
    session.navigate(&mut nav, |n| {
        n.select(Section::Actors);
        true
    });
    let model = session.actors();
    let mut list = model.list();
    model.show();
    let generation = view_until(&mut list, |list| list.rows.len() == 1).await.generation;
    served(&mut server, ROOTS).await;

    let pages: Arc<dyn Pages> = session.clone();
    let live = Live::new(&Handle::current(), Arc::downgrade(&pages), Limits::DEFAULT.debounce);
    let received = Received::default();
    live.show(Section::Actors, "", session.visit().1);
    live.use_source(Some(Tap { client: server.client.clone(), received: received.clone() }));
    let mut status = live.status();
    view_until(&mut status, |status| *status == LiveStatus::Live).await;
    assert!(!received.lock().unwrap().is_empty(), "Live from the first valid event");
    // The first resync refreshes the page once; then it is idle.
    served(&mut server, ROOTS).await;
    sleep(Duration::from_millis(300)).await;
    while server.events.try_recv().is_ok() {}

    actors.create(&Counter { id: "b".into() }, &json!({})).await;
    let started = served(&mut server, ROOTS).await;
    let hinted = received
        .lock()
        .unwrap()
        .iter()
        .find(|(_, change)| matches!(change, Change::Changed(address) if address.as_str() == "counter/b"))
        .map(|(at, _)| *at)
        .expect("a hint for counter/b");
    assert!(started - hinted < Duration::from_secs(1), "started {:?} after the hint", started - hinted);
    let view = view_until(&mut list, |list| list.rows.len() == 2).await;
    assert_eq!(view.generation, generation, "a soft refresh keeps the list's generation");
    assert_eq!(*status.borrow(), LiveStatus::Live);
}

/// Counts a server's `/stats` reads and the change streams it opened.
#[derive(Clone, Default)]
struct Reads {
    stats: Arc<AtomicUsize>,
    streams: Arc<AtomicUsize>,
}

impl Reads {
    fn rule(&self) -> Rule {
        let reads = self.clone();
        Arc::new(move |target| {
            if target == "/stats" {
                reads.stats.fetch_add(1, Ordering::SeqCst);
            } else if target.starts_with("/changes") {
                reads.streams.fetch_add(1, Ordering::SeqCst);
            }
            None
        })
    }

    fn stats(&self) -> usize {
        self.stats.load(Ordering::SeqCst)
    }

    async fn until_stats(&self, count: usize) {
        timeout(Duration::from_secs(10), async {
            while self.stats() < count {
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the /stats reads in time");
    }
}

/// Scenario: A context switch on the Overview, with two servers.
#[tokio::test]
async fn a_context_switch_on_the_overview_reloads_the_new_server_only_from_its_own_stream() {
    let (old_actors, new_actors) = (actors().await, actors().await);
    let (old, new) = (Reads::default(), Reads::default());
    let old_server = serve(&old_actors, old.rule(), Duration::ZERO).await;
    let new_server = serve(&new_actors, new.rule(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let env = config(&dir, &[("old", &old_server.url), ("new", &new_server.url)]);
    let session = Arc::new(Session::new(env, Handle::current(), Limits::DEFAULT));
    let pages: Arc<dyn Pages> = session.clone();
    let live = Live::new(&Handle::current(), Arc::downgrade(&pages), Limits::DEFAULT.debounce);
    let follow = || {
        let (visit, context, client) = session.live_target();
        live.follow(context, client, Section::Overview, "", visit);
    };
    let mut status = live.status();

    session.use_context(Some("old".into()));
    follow();
    view_until(&mut status, |status| *status == LiveStatus::Live).await;
    // The Overview's own read, then the one of the first resync.
    old.until_stats(2).await;

    session.use_context(Some("new".into()));
    follow();
    new.until_stats(2).await;
    sleep(Duration::from_millis(1500)).await;
    assert_eq!(new.stats(), 2, "the new server: its Overview read and its own stream's resync, nothing more");
    assert_eq!(old.stats(), 2, "the old server: nothing after the switch");
    assert_eq!(old.streams.load(Ordering::SeqCst), 1, "no stream reopened on the old server");
    assert_eq!(new.streams.load(Ordering::SeqCst), 1);
}
