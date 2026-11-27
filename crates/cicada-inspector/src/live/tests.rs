//! The live model on virtual time against a channel change source.

use std::time::Duration;

use cicada::ActorAddress;
use cicada_client::Change;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};

use super::channel::{Channel, Opened};
use super::{Live, LiveStatus};
use crate::model::Section;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Lets the model's task handle what was sent, a millisecond of virtual time later.
async fn settle() {
    sleep(Duration::from_millis(1)).await;
}

fn address(text: &str) -> ActorAddress {
    text.parse().unwrap()
}

/// A model on a channel source, showing `section` with `search`, and its first stream.
async fn start(section: Section, search: &str) -> (Live<Channel>, Channel, mpsc::UnboundedReceiver<Opened>, Opened) {
    let (source, mut opened) = Channel::new();
    let live = Live::new(&Handle::current(), DEBOUNCE);
    live.show(section, search);
    live.use_source(Some(source.clone()));
    let first = opened.recv().await.unwrap();
    (live, source, opened, first)
}

async fn no_try(opened: &mut mpsc::UnboundedReceiver<Opened>) {
    settle().await;
    assert!(opened.try_recv().is_err(), "no stream was opened");
}

#[tokio::test(start_paused = true)]
async fn one_stream_follows_the_page_and_its_prefix() {
    let (live, _source, mut opened, first) = start(Section::Actors, "counter/").await;
    assert_eq!(first.prefix, "counter/");

    // A new prefix waits for the search to settle; the stream before stays open meanwhile.
    live.show(Section::Actors, "counter/a");
    sleep(DEBOUNCE - Duration::from_millis(10)).await;
    assert!(opened.try_recv().is_err());
    assert!(!first.hints.is_closed());
    let next = opened.recv().await.unwrap();
    assert_eq!(next.prefix, "counter/a");
    assert!(first.hints.is_closed(), "the stream of the old prefix is closed");

    // A page switch opens the next stream at once; the Overview follows every actor.
    let switched = Instant::now();
    live.show(Section::Overview, "");
    let overview = opened.recv().await.unwrap();
    assert_eq!((overview.prefix.as_str(), overview.at), ("", switched));
    assert!(next.hints.is_closed());

    // Messages, Running Turns and Outcomes subscribe to their search prefix too.
    for section in [Section::Messages, Section::RunningTurns, Section::Outcomes] {
        live.show(section, "sync/");
        assert_eq!(opened.recv().await.unwrap().prefix, "sync/");
    }
}

#[tokio::test(start_paused = true)]
async fn a_local_filter_leaves_the_stream_alone() {
    for section in [Section::Types, Section::Watches, Section::Nodes] {
        let (live, _source, mut opened, first) = start(section, "").await;
        assert_eq!(first.prefix, "");
        live.show(section, "counter");
        sleep(DEBOUNCE * 2).await;
        no_try(&mut opened).await;
        assert!(!first.hints.is_closed());
    }
}

#[tokio::test(start_paused = true)]
async fn a_prefix_typed_back_before_the_debounce_keeps_the_stream() {
    let (live, _source, mut opened, first) = start(Section::Actors, "counter/").await;
    live.show(Section::Actors, "counter/a");
    sleep(Duration::from_millis(100)).await;
    live.show(Section::Actors, "counter/");
    sleep(DEBOUNCE * 2).await;
    no_try(&mut opened).await;
    assert!(!first.hints.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_context_switch_and_a_close_end_the_stream() {
    let (live, _source, mut opened, first) = start(Section::Actors, "").await;
    let (other, mut other_opened) = Channel::new();
    live.use_source(Some(other));
    let next = other_opened.recv().await.unwrap();
    assert!(first.hints.is_closed());
    no_try(&mut opened).await;

    live.use_source(None);
    settle().await;
    assert!(next.hints.is_closed());
    assert_eq!(*live.status().borrow(), LiveStatus::Off);

    let (live, _source, _opened, stream) = start(Section::Actors, "").await;
    drop(live);
    settle().await;
    assert!(stream.hints.is_closed(), "closing the window closes the stream");
}

#[tokio::test(start_paused = true)]
async fn live_shows_from_the_first_valid_event() {
    let (live, _source, _opened, first) = start(Section::Actors, "").await;
    let status = live.status();
    settle().await;
    assert_eq!(*status.borrow(), LiveStatus::Connecting, "an open stream without events is not live yet");
    first.hints.send(Ok(Change::Changed(address("counter/a")))).unwrap();
    settle().await;
    assert_eq!(*status.borrow(), LiveStatus::Live);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_stream_reconnects_with_backoff() {
    let (live, source, mut opened, first) = start(Section::Actors, "").await;
    let status = live.status();
    source.fail_next(["refused"; 7]);
    let ended = Instant::now();
    drop(first.hints);
    settle().await;
    assert!(matches!(&*status.borrow(), LiveStatus::Reconnecting { reason, .. } if reason.contains("ended")));
    let mut last = ended;
    let mut tried = None;
    for wait in [1, 2, 4, 8, 16, 30, 30, 30] {
        let tried = tried.insert(opened.recv().await.unwrap());
        assert_eq!(tried.at - last, Duration::from_secs(wait));
        last = tried.at;
        settle().await;
        if tried.failed.is_some() {
            assert!(matches!(&*status.borrow(), LiveStatus::Reconnecting { reason, .. } if reason == "refused"));
        }
    }
    // The eighth try opened: the pill says Reconnecting until its first event.
    assert!(matches!(&*status.borrow(), LiveStatus::Reconnecting { .. }));
}

#[tokio::test(start_paused = true)]
async fn the_backoff_starts_again_after_a_minute_of_events() {
    let (_live, source, mut opened, first) = start(Section::Actors, "").await;
    source.fail_next(["refused", "refused"]);
    drop(first.hints);
    let tries: Vec<_> = [opened.recv().await.unwrap(), opened.recv().await.unwrap(), opened.recv().await.unwrap()]
        .into_iter()
        .collect();
    assert_eq!(tries[2].at - tries[1].at, Duration::from_secs(4));
    let open = &tries[2];

    // A stream that stays up 59 s after its first event keeps the backoff where it was.
    open.hints.send(Ok(Change::Resync)).unwrap();
    sleep(Duration::from_secs(59)).await;
    let ended = Instant::now();
    drop(tries);
    let short = opened.recv().await.unwrap();
    assert_eq!(short.at - ended, Duration::from_secs(8));

    // One that stays up 60 s starts again from 1 s.
    short.hints.send(Ok(Change::Resync)).unwrap();
    sleep(Duration::from_secs(60)).await;
    let ended = Instant::now();
    drop(short);
    let steady = opened.recv().await.unwrap();
    assert_eq!(steady.at - ended, Duration::from_secs(1));
}
