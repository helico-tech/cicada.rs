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
    live.show(Section::Actors, "");
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
