use std::task::Poll;

use futures::{StreamExt, pin_mut, poll, stream};
use tokio::time::advance;

use super::*;

struct TestChunks<S>(Pin<Box<S>>);

impl<S, B> ChunkSource for TestChunks<S>
where
    S: Stream<Item = Result<B, ChangeStreamError>> + Send,
    B: AsRef<[u8]> + Send + 'static,
{
    async fn next_chunk(&mut self) -> Result<Option<impl AsRef<[u8]> + Send + use<S, B>>, ChangeStreamError> {
        self.0.next().await.transpose()
    }
}

fn read_chunks<B>(chunks: impl Stream<Item = Result<B, ChangeStreamError>> + Send + 'static) -> ChangeStream
where
    B: AsRef<[u8]> + Send + 'static,
{
    read_stream(TestChunks(Box::pin(chunks)))
}

/// Let the reader run, without advancing virtual time or waiting indefinitely for a missing result.
async fn ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let start = std::time::Instant::now();
    loop {
        if let Poll::Ready(result) = poll!(&mut future) {
            return result;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "reader made no progress within 5 wall-clock seconds");
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn parses_fragmented_event_stream_fields_and_discards_an_incomplete_event() {
    let chunks: &[&[u8]] = &[
        b"\xef",
        b"\xbb\xbfevent: changed\r",
        b"\ndata: {\rdata\ndata: \"address\": \"counter/a\"\n",
        b"data: }\r\n: a comment\r",
        b"\revent: future\ndata: ignored\n\n",
        b"event: changed\nevent\ndata: ignored\n\n",
        b"event:  changed\ndata: ignored\n\n",
        b"event: changed\n\n",
        b"event: resync\ndata: {}\r\r",
        b"\nevent: changed\ndata: {\"address\": \"counter/incomplete\"}\n",
    ];
    let mut events = read_chunks(stream::iter(chunks.iter().copied().map(Ok)));
    assert!(
        matches!(ready(events.next()).await, Some(Ok(Change::Changed(address))) if address.as_str() == "counter/a")
    );
    assert!(matches!(ready(events.next()).await, Some(Ok(Change::Resync))));
    assert!(ready(events.next()).await.is_none());
    assert!(ready(events.next()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_line_can_be_64_kib_but_one_more_byte_ends_the_stream() {
    let mut line = BOM.to_vec();
    line.push(b':');
    line.resize(BOM.len() + MAX_LINE, b'x');
    let complete = b"\r\nevent: resync\ndata: {}\n\n".to_vec();
    let mut events = read_chunks(stream::iter([Ok(line.clone()), Ok(complete)]));
    assert!(matches!(ready(events.next()).await, Some(Ok(Change::Resync))));
    assert!(ready(events.next()).await.is_none());

    let mut events = read_chunks(stream::iter([Ok(line), Ok(vec![b'x'])]));
    assert!(matches!(ready(events.next()).await, Some(Err(ChangeStreamError::Malformed("line exceeds 64 KiB")))));
    assert!(ready(events.next()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn an_events_joined_data_is_limited_to_64_kib_even_without_a_terminating_blank_line() {
    let first_data = r#"{"address":"counter/a""#;
    let first = format!("event: changed\ndata: {first_data}\n");
    let remaining = MAX_EVENT_DATA - first_data.len() - "\n}".len();
    let second = format!("data: {}}}\n", " ".repeat(remaining));
    let mut events = read_chunks(stream::iter([Ok(first.clone()), Ok(format!("{second}\n"))]));
    assert!(
        matches!(ready(events.next()).await, Some(Ok(Change::Changed(address))) if address.as_str() == "counter/a")
    );
    assert!(ready(events.next()).await.is_none());

    let second = format!("data: {}}}\n", " ".repeat(remaining + 1));
    let mut events = read_chunks(stream::iter([Ok(first), Ok(second)]).chain(stream::pending()));
    assert!(matches!(ready(events.next()).await, Some(Err(ChangeStreamError::Malformed("event data exceeds 64 KiB")))));
    assert!(ready(events.next()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn changed_requires_a_valid_address_and_keeps_newlines_between_data_lines() {
    let malformed: &[&[u8]] = &[
        b"event: changed\ndata: {}\n\n",
        b"event: changed\ndata: {\"address\": \"not an address\"}\n\n",
        b"event: changed\ndata: {\"address\": \"counter/\ndata: a\"}\n\n",
        b"event: changed\ndata\n\n",
    ];
    for frame in malformed {
        let mut events = read_chunks(stream::iter([Ok(*frame), Ok(b"event: resync\ndata: {}\n\n")]));
        assert!(matches!(ready(events.next()).await, Some(Err(ChangeStreamError::Malformed(_)))));
        assert!(ready(events.next()).await.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn a_silent_stream_is_idle_at_45_seconds() {
    let source = TestChunks(Box::pin(stream::pending::<Result<&[u8], _>>()));
    let (changes, _received) = mpsc::channel(1);
    let read = read_changes(source, Instant::now(), &changes);
    pin_mut!(read);
    assert!(poll!(&mut read).is_pending());
    advance(Duration::from_secs(44)).await;
    assert!(poll!(&mut read).is_pending());
    advance(Duration::from_secs(1)).await;
    assert!(matches!(poll!(read), Poll::Ready(Err(ChangeStreamError::Idle))));
}

#[tokio::test(start_paused = true)]
async fn an_empty_read_does_not_reset_idle() {
    let chunks = stream::once(async {
        tokio::time::sleep(Duration::from_secs(40)).await;
        Ok(&b""[..])
    })
    .chain(stream::pending());
    let (changes, _received) = mpsc::channel(1);
    let read = read_changes(TestChunks(Box::pin(chunks)), Instant::now(), &changes);
    pin_mut!(read);
    assert!(poll!(&mut read).is_pending());
    advance(Duration::from_secs(40)).await;
    assert!(poll!(&mut read).is_pending());
    advance(Duration::from_secs(5)).await;
    assert!(matches!(poll!(read), Poll::Ready(Err(ChangeStreamError::Idle))));
}

#[tokio::test(start_paused = true)]
async fn a_slow_consumer_gets_the_next_hint_when_its_chunk_is_already_waiting() {
    let chunks: [&[u8]; 2] = [b"event: resync\ndata: {}\n\n", b"event: changed\ndata: {\"address\":\"counter/a\"}\n\n"];
    let mut events = read_chunks(stream::iter(chunks.map(Ok)));
    assert!(matches!(ready(events.next()).await, Some(Ok(Change::Resync))));
    advance(Duration::from_secs(46)).await;
    assert!(
        matches!(ready(events.next()).await, Some(Ok(Change::Changed(address))) if address.as_str() == "counter/a")
    );
    assert!(ready(events.next()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_ready_chunk_wins_over_idle_when_the_full_delivery_buffer_drains() {
    let (third_read, observed) = tokio::sync::oneshot::channel();
    let first: [&[u8]; 2] = [b"event: resync\ndata: {}\n\n", b"event: changed\ndata: {\"address\":\"counter/a\"}\n\n"];
    let third = stream::once(async {
        third_read.send(()).unwrap();
        Ok(&b"event: changed\ndata: {\"address\":\"counter/b\"}\n\n"[..])
    });
    let last = Ok(&b"event: changed\ndata: {\"address\":\"counter/c\"}\n\n"[..]);
    let mut events = read_chunks(stream::iter(first.map(Ok)).chain(third).chain(stream::iter([last])));
    assert!(matches!(ready(events.next()).await, Some(Ok(Change::Resync))));
    ready(observed).await.unwrap();

    advance(Duration::from_secs(46)).await;
    for expected in ["counter/a", "counter/b", "counter/c"] {
        assert!(
            matches!(ready(events.next()).await, Some(Ok(Change::Changed(address))) if address.as_str() == expected)
        );
    }
    assert!(ready(events.next()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn a_keep_alive_resets_idle_at_arrival_between_next_calls() {
    let started = Instant::now();
    let (sent, arrived) = tokio::sync::oneshot::channel();
    let keep_alive = stream::once(async move {
        sleep_until(started + Duration::from_secs(40)).await;
        sent.send(Instant::now()).unwrap();
        Ok(&b": keep-alive\n\n"[..])
    });
    let chunks = stream::iter([Ok(&b"event: resync\ndata: {}\n\n"[..])]).chain(keep_alive).chain(stream::pending());
    let mut events = read_chunks(chunks);
    assert!(matches!(ready(events.next()).await, Some(Ok(Change::Resync))));

    advance(Duration::from_secs(40)).await;
    assert_eq!(ready(arrived).await.unwrap(), started + Duration::from_secs(40));
    advance(Duration::from_secs(44)).await;
    assert!(poll!(events.next()).is_pending());
    advance(Duration::from_secs(1)).await;
    assert!(matches!(ready(events.next()).await, Some(Err(ChangeStreamError::Idle))));
    assert!(ready(events.next()).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn keep_alives_reset_idle_but_the_stream_expires_at_24_hours() {
    let spacing = Duration::from_secs(40);
    let chunks = stream::unfold(Instant::now() + spacing, move |next| async move {
        sleep_until(next).await;
        Some((Ok(&b": keep-alive\n\n"[..]), next + spacing))
    });
    let (changes, _received) = mpsc::channel(1);
    let read = read_changes(TestChunks(Box::pin(chunks)), Instant::now(), &changes);
    let wall_start = std::time::Instant::now();
    pin_mut!(read);
    for _ in 0..LIFETIME.as_secs() / spacing.as_secs() {
        assert!(wall_start.elapsed() < Duration::from_secs(5));
        assert!(poll!(&mut read).is_pending());
        advance(spacing).await;
    }
    assert!(matches!(poll!(read), Poll::Ready(Err(ChangeStreamError::Lifetime))));
}
