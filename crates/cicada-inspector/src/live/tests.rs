//! The live model on virtual time against a channel change source.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::ActorAddress;
use cicada_client::Change;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};

use super::channel::{Channel, Opened};
use super::{FALLBACK, GATHER, Live, LiveStatus, Pages, Reloaded};
use crate::model::Section;
use crate::refresh::Done;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Lets the model's task handle what was sent, a millisecond of virtual time later.
async fn settle() {
    sleep(Duration::from_millis(1)).await;
}

fn address(text: &str) -> ActorAddress {
    text.parse().unwrap()
}

/// Pages whose soft refreshes are recorded and take `length` each, or are skipped at once without one.
#[derive(Default)]
struct FakePages {
    refreshes: Mutex<Vec<(Section, Instant)>>,
    cancels: Mutex<Vec<Section>>,
    pane: Mutex<Option<ActorAddress>>,
    length: Mutex<Option<Duration>>,
    /// How long a manual reload takes.
    reload: Mutex<Duration>,
}

impl FakePages {
    fn new(length: Option<Duration>) -> Arc<Self> {
        Arc::new(Self { length: Mutex::new(length), ..Self::default() })
    }

    fn count(&self) -> usize {
        self.refreshes.lock().unwrap().len()
    }

    fn times(&self) -> Vec<Instant> {
        self.refreshes.lock().unwrap().iter().map(|(_, at)| *at).collect()
    }
}

impl Pages for FakePages {
    fn soft_refresh(&self, section: Section) -> Done {
        self.refreshes.lock().unwrap().push((section, Instant::now()));
        let Some(length) = *self.length.lock().unwrap() else { return Done::ready() };
        let (ended, done) = Done::channel();
        tokio::spawn(async move {
            sleep(length).await;
            drop(ended);
        });
        done
    }

    fn cancel_soft_refresh(&self, section: Section) {
        self.cancels.lock().unwrap().push(section);
    }

    fn pane_actor(&self, _: Section) -> Option<ActorAddress> {
        self.pane.lock().unwrap().clone()
    }

    fn reloaded(&self, _: Section) -> Reloaded {
        Box::pin(sleep(*self.reload.lock().unwrap()))
    }
}

/// A refresh that takes 100 ms.
const LENGTH: Option<Duration> = Some(Duration::from_millis(100));

type Fixture = (Live<Channel>, Channel, mpsc::UnboundedReceiver<Opened>, Opened, Arc<FakePages>);

/// A model on a channel source and fake pages whose refreshes take `LENGTH`, showing `section` with `search`, and
/// its first stream.
async fn start(section: Section, search: &str) -> Fixture {
    start_with(section, search, FakePages::new(LENGTH)).await
}

async fn start_with(section: Section, search: &str, pages: Arc<FakePages>) -> Fixture {
    let (source, mut opened) = Channel::new();
    let weak = Arc::downgrade(&pages);
    let live = Live::new(&Handle::current(), weak, DEBOUNCE);
    live.show(section, search);
    live.use_source(Some(source.clone()));
    let first = opened.recv().await.unwrap();
    (live, source, opened, first, pages)
}

fn hint(opened: &Opened, address_text: &str) {
    opened.hints.send(Ok(Change::Changed(address(address_text)))).unwrap();
}

async fn no_try(opened: &mut mpsc::UnboundedReceiver<Opened>) {
    settle().await;
    assert!(opened.try_recv().is_err(), "no stream was opened");
}

#[tokio::test(start_paused = true)]
async fn one_stream_follows_the_page_and_its_prefix() {
    let (live, _source, mut opened, first, _pages) = start(Section::Actors, "counter/").await;
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
        let (live, _source, mut opened, first, _pages) = start(section, "").await;
        assert_eq!(first.prefix, "");
        live.show(section, "counter");
        sleep(DEBOUNCE * 2).await;
        no_try(&mut opened).await;
        assert!(!first.hints.is_closed());
    }
}

#[tokio::test(start_paused = true)]
async fn a_prefix_typed_back_before_the_debounce_keeps_the_stream() {
    let (live, _source, mut opened, first, _pages) = start(Section::Actors, "counter/").await;
    live.show(Section::Actors, "counter/a");
    sleep(Duration::from_millis(100)).await;
    live.show(Section::Actors, "counter/");
    sleep(DEBOUNCE * 2).await;
    no_try(&mut opened).await;
    assert!(!first.hints.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_context_switch_and_a_close_end_the_stream() {
    let (live, _source, mut opened, first, _pages) = start(Section::Actors, "").await;
    let (other, mut other_opened) = Channel::new();
    live.use_source(Some(other));
    let next = other_opened.recv().await.unwrap();
    assert!(first.hints.is_closed());
    no_try(&mut opened).await;

    live.use_source(None);
    settle().await;
    assert!(next.hints.is_closed());
    assert_eq!(*live.status().borrow(), LiveStatus::Off);

    let (live, _source, _opened, stream, _pages) = start(Section::Actors, "").await;
    drop(live);
    settle().await;
    assert!(stream.hints.is_closed(), "closing the window closes the stream");
}

#[tokio::test(start_paused = true)]
async fn live_shows_from_the_first_valid_event() {
    let (live, _source, _opened, first, _pages) = start(Section::Actors, "").await;
    let status = live.status();
    settle().await;
    assert_eq!(*status.borrow(), LiveStatus::Connecting, "an open stream without events is not live yet");
    first.hints.send(Ok(Change::Changed(address("counter/a")))).unwrap();
    settle().await;
    assert_eq!(*status.borrow(), LiveStatus::Live);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_stream_reconnects_with_backoff() {
    let (live, source, mut opened, first, _pages) = start(Section::Actors, "").await;
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
    let (_live, source, mut opened, first, _pages) = start(Section::Actors, "").await;
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

#[tokio::test(start_paused = true)]
async fn the_first_resync_of_an_open_and_a_reconnect_each_refresh_once() {
    let (_live, _source, mut opened, first, pages) = start(Section::Actors, "").await;
    sleep(FALLBACK - Duration::from_secs(1)).await;
    assert_eq!(pages.count(), 0, "a stream with no event causes no refresh");

    let resync = Instant::now();
    first.hints.send(Ok(Change::Resync)).unwrap();
    sleep(Duration::from_secs(1)).await;
    assert_eq!(pages.times(), [resync + GATHER]);

    drop(first);
    let second = opened.recv().await.unwrap();
    sleep(Duration::from_secs(1)).await;
    assert_eq!(pages.count(), 1, "the reconnect's opening is no trigger");
    second.hints.send(Ok(Change::Resync)).unwrap();
    sleep(Duration::from_secs(1)).await;
    assert_eq!(pages.count(), 2, "its resync is");
}

#[tokio::test(start_paused = true)]
async fn fifty_hints_in_200_ms_run_one_refresh() {
    let (_live, _source, _opened, first, pages) = start(Section::Actors, "counter/").await;
    let started = Instant::now();
    for n in 0..50 {
        hint(&first, &format!("counter/{n}"));
        sleep(Duration::from_millis(4)).await;
    }
    sleep(Duration::from_secs(5)).await;
    assert_eq!(pages.times(), [started + GATHER]);
    assert_eq!(pages.refreshes.lock().unwrap()[0].0, Section::Actors);
}

#[tokio::test(start_paused = true)]
async fn hints_during_a_refresh_run_exactly_one_more_after_it() {
    let length = Duration::from_secs(2);
    let (_live, _source, _opened, first, pages) = start_with(Section::Actors, "", FakePages::new(Some(length))).await;
    let started = Instant::now();
    hint(&first, "counter/a");
    sleep(GATHER + Duration::from_millis(500)).await;
    for n in 0..10 {
        hint(&first, &format!("counter/{n}"));
        sleep(Duration::from_millis(100)).await;
    }
    sleep(Duration::from_secs(10)).await;
    let first_start = started + GATHER;
    assert_eq!(pages.times(), [first_start, first_start + length], "the next starts once the refresh ends");
}

#[tokio::test(start_paused = true)]
async fn a_batch_over_1000_addresses_counts_as_everything() {
    for (count, refreshes) in [(1000, 0), (1001, 1)] {
        let (_live, _source, _opened, first, pages) = start(Section::Actors, "counter/").await;
        for n in 0..count {
            hint(&first, &format!("sync/{n}"));
        }
        sleep(Duration::from_secs(1)).await;
        assert_eq!(pages.count(), refreshes, "{count} addresses outside the prefix");
    }
}

#[tokio::test(start_paused = true)]
async fn a_batch_outside_the_prefix_and_the_pane_starts_no_refresh_on_a_paged_page() {
    for section in [Section::Actors, Section::Messages, Section::RunningTurns, Section::Outcomes] {
        let (_live, _source, _opened, first, pages) = start(section, "counter/").await;
        hint(&first, "sync/a");
        sleep(Duration::from_secs(1)).await;
        assert_eq!(pages.count(), 0, "{section:?}");
        *pages.pane.lock().unwrap() = Some(address("sync/a"));
        hint(&first, "sync/a");
        sleep(Duration::from_secs(1)).await;
        assert_eq!(pages.count(), 1, "{section:?}: the pane's actor");
        hint(&first, "counter/a");
        sleep(Duration::from_secs(1)).await;
        assert_eq!(pages.count(), 2, "{section:?}: under the prefix");
    }
}

#[tokio::test(start_paused = true)]
async fn any_hint_refreshes_the_other_pages() {
    for section in [Section::Overview, Section::Nodes, Section::Types, Section::Watches] {
        let (_live, _source, _opened, first, pages) = start(section, "local filter").await;
        hint(&first, "sync/a");
        sleep(Duration::from_secs(1)).await;
        assert_eq!(pages.count(), 1, "{section:?}");
        assert_eq!(pages.refreshes.lock().unwrap()[0].0, section);
    }
}

#[tokio::test(start_paused = true)]
async fn the_timer_refreshes_30_s_after_the_last_refresh_ended() {
    let (_live, _source, _opened, first, pages) = start(Section::Actors, "").await;
    let resync = Instant::now();
    first.hints.send(Ok(Change::Resync)).unwrap();
    sleep(FALLBACK * 2 + Duration::from_secs(1)).await;
    let length = LENGTH.unwrap();
    let after_resync = resync + GATHER;
    let timer = after_resync + length + FALLBACK;
    assert_eq!(pages.times(), [after_resync, timer, timer + length + FALLBACK]);
}

#[tokio::test(start_paused = true)]
async fn a_skipped_refresh_arms_the_timer_again() {
    let (_live, _source, _opened, _first, pages) = start_with(Section::Actors, "", FakePages::new(None)).await;
    let opened = Instant::now();
    sleep(FALLBACK * 3 + Duration::from_secs(1)).await;
    assert_eq!(pages.times(), [opened + FALLBACK, opened + FALLBACK * 2, opened + FALLBACK * 3]);
}

#[tokio::test(start_paused = true)]
async fn the_timer_runs_while_reconnecting() {
    let (live, source, _opened, first, pages) = start(Section::Actors, "").await;
    source.fail_next(["refused"; 20]);
    let dropped = Instant::now();
    drop(first);
    sleep(FALLBACK + Duration::from_secs(1)).await;
    assert!(matches!(*live.status().borrow(), LiveStatus::Reconnecting { .. }));
    assert_eq!(pages.times(), [dropped + FALLBACK]);
}

#[tokio::test(start_paused = true)]
async fn pause_closes_the_stream_drops_the_batch_and_cancels_a_refresh_in_flight() {
    let length = Duration::from_secs(2);
    let (live, _source, mut opened, first, pages) =
        start_with(Section::Messages, "", FakePages::new(Some(length))).await;
    let status = live.status();
    hint(&first, "counter/a");
    sleep(GATHER + Duration::from_millis(100)).await;
    assert_eq!(pages.count(), 1, "a refresh runs");
    hint(&first, "counter/b");
    settle().await;
    live.pause();
    settle().await;
    assert_eq!(*pages.cancels.lock().unwrap(), [Section::Messages]);
    assert!(first.hints.is_closed());
    assert_eq!(*status.borrow(), LiveStatus::Paused);

    // Nothing reloads by itself: no batch, no timer, no stream, also across a page switch and a context switch.
    live.show(Section::Actors, "");
    live.use_source(Some(Channel::new().0));
    sleep(FALLBACK * 4).await;
    assert_eq!(pages.count(), 1);
    assert!(opened.try_recv().is_err());
    assert_eq!(*status.borrow(), LiveStatus::Paused);
}

#[tokio::test(start_paused = true)]
async fn resume_opens_a_stream_whose_first_resync_refreshes_once() {
    let (live, _source, mut opened, first, pages) = start(Section::Actors, "").await;
    live.pause();
    settle().await;
    assert!(first.hints.is_closed());
    live.resume();
    let resumed = opened.recv().await.unwrap();
    settle().await;
    assert_eq!(*live.status().borrow(), LiveStatus::Connecting);
    assert_eq!(pages.count(), 0, "the opening is no trigger");
    let resync = Instant::now();
    resumed.hints.send(Ok(Change::Resync)).unwrap();
    sleep(Duration::from_secs(5)).await;
    assert_eq!(pages.times(), [resync + GATHER]);
    assert_eq!(*live.status().borrow(), LiveStatus::Live);
}

#[tokio::test(start_paused = true)]
async fn a_manual_refresh_arms_the_timer_again_unless_paused() {
    let (live, _source, _opened, _first, pages) = start(Section::Actors, "").await;
    let opened = Instant::now();
    sleep(Duration::from_secs(20)).await;
    live.refreshed(Section::Actors);
    sleep(FALLBACK + Duration::from_secs(1)).await;
    assert_eq!(pages.times(), [opened + Duration::from_secs(20) + FALLBACK]);

    live.pause();
    live.refreshed(Section::Actors);
    sleep(FALLBACK * 3).await;
    assert_eq!(pages.count(), 1, "⌘R while paused arms no timer");
}

#[test]
fn the_pill_says_what_each_state_means() {
    let at = chrono::Utc::now();
    let reconnecting = LiveStatus::Reconnecting { at, reason: "refused".into() };
    let titles =
        [LiveStatus::Off, LiveStatus::Live, reconnecting.clone(), LiveStatus::Paused].map(|status| status.title());
    assert_eq!(titles, ["Offline", "Live", "Reconnecting", "Paused"]);
    let tip = reconnecting.tooltip();
    assert!(tip.contains("refused") && tip.contains(&at.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()));
    assert!(LiveStatus::Paused.tooltip().contains("⌘R"));
    assert!(LiveStatus::Live.tooltip().contains("30 s"));
}

#[tokio::test(start_paused = true)]
async fn the_timer_after_a_manual_reload_counts_from_its_end() {
    let pages = FakePages::new(LENGTH);
    *pages.reload.lock().unwrap() = Duration::from_secs(10);
    let (live, _source, _opened, _first, pages) = start_with(Section::Actors, "", pages).await;
    sleep(Duration::from_secs(20)).await;
    let reload = Instant::now();
    live.refreshed(Section::Actors);
    sleep(FALLBACK * 2).await;
    assert_eq!(pages.times()[0], reload + Duration::from_secs(10) + FALLBACK);
}

#[tokio::test(start_paused = true)]
async fn a_prefix_waiting_for_the_debounce_survives_a_pause_and_a_context_switch() {
    let (live, _source, mut opened, _first, _pages) = start(Section::Actors, "counter/").await;
    live.show(Section::Actors, "counter/a");
    settle().await;
    live.pause();
    live.resume();
    assert_eq!(opened.recv().await.unwrap().prefix, "counter/a", "resume follows the prefix typed last");

    live.show(Section::Actors, "counter/b");
    settle().await;
    let (other, mut other_opened) = Channel::new();
    live.use_source(Some(other));
    assert_eq!(other_opened.recv().await.unwrap().prefix, "counter/b", "the new context follows it too");
    sleep(DEBOUNCE * 2).await;
    no_try(&mut other_opened).await;
}

#[tokio::test(start_paused = true)]
async fn pause_cancels_the_refresh_before_it_returns_and_no_batch_starts_after_it() {
    let length = Duration::from_secs(2);
    let (live, _source, mut opened, first, pages) =
        start_with(Section::Watches, "", FakePages::new(Some(length))).await;
    hint(&first, "counter/a");
    sleep(GATHER + Duration::from_millis(100)).await;
    assert_eq!(pages.count(), 1);
    live.pause();
    assert_eq!(*pages.cancels.lock().unwrap(), [Section::Watches], "cancelled before the model's task ran");
    live.resume();
    let resumed = opened.recv().await.unwrap();
    settle().await;

    // A batch that falls due as the click comes: started before it and cancelled, or not started at all.
    hint(&resumed, "counter/b");
    sleep(GATHER).await;
    let at_click = pages.count();
    live.pause();
    sleep(Duration::from_secs(5)).await;
    assert_eq!(pages.count(), at_click, "nothing starts after the click");
    assert_eq!(pages.cancels.lock().unwrap().len(), at_click, "what started before it was cancelled");
}

#[tokio::test(start_paused = true)]
async fn nothing_starts_once_the_model_is_stopped() {
    let (live, _source, _opened, first, pages) = start(Section::Actors, "").await;
    hint(&first, "counter/a");
    settle().await;
    live.stop();
    sleep(FALLBACK * 2).await;
    assert_eq!(pages.count(), 0);
}
