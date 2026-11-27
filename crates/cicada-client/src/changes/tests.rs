use std::task::Poll;

use futures::{pin_mut, poll, stream};
use tokio::time::advance;

use super::*;

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
    let mut events = read_stream(stream::iter(chunks.iter().copied().map(Ok)));
    assert!(matches!(poll!(events.next()), Poll::Ready(Some(Ok(Change::Changed(address))))
        if address.as_str() == "counter/a"));
    assert!(matches!(poll!(events.next()), Poll::Ready(Some(Ok(Change::Resync)))));
    assert!(matches!(poll!(events.next()), Poll::Ready(None)));
    assert!(matches!(poll!(events.next()), Poll::Ready(None)));
}

#[tokio::test(start_paused = true)]
async fn a_line_can_be_64_kib_but_one_more_byte_ends_the_stream() {
    let mut line = BOM.to_vec();
    line.push(b':');
    line.resize(BOM.len() + MAX_LINE, b'x');
    let complete = b"\r\nevent: resync\ndata: {}\n\n".to_vec();
    let mut events = read_stream(stream::iter([Ok(line.clone()), Ok(complete)]));
    assert!(matches!(poll!(events.next()), Poll::Ready(Some(Ok(Change::Resync)))));
    assert!(matches!(poll!(events.next()), Poll::Ready(None)));

    let mut events = read_stream(stream::iter([Ok(line), Ok(vec![b'x'])]));
    assert!(matches!(
        poll!(events.next()),
        Poll::Ready(Some(Err(ChangeStreamError::Malformed("line exceeds 64 KiB"))))
    ));
    assert!(matches!(poll!(events.next()), Poll::Ready(None)));
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
        let mut events = read_stream(stream::iter([Ok(*frame), Ok(b"event: resync\ndata: {}\n\n")]));
        assert!(matches!(poll!(events.next()), Poll::Ready(Some(Err(ChangeStreamError::Malformed(_))))));
        assert!(matches!(poll!(events.next()), Poll::Ready(None)));
    }
}

#[tokio::test(start_paused = true)]
async fn a_silent_stream_is_idle_at_45_seconds() {
    let mut events = read_stream(stream::pending::<Result<&[u8], _>>());
    let next = events.next();
    pin_mut!(next);
    assert!(poll!(&mut next).is_pending());
    advance(Duration::from_secs(44)).await;
    assert!(poll!(&mut next).is_pending());
    advance(Duration::from_secs(1)).await;
    assert!(matches!(poll!(next), Poll::Ready(Some(Err(ChangeStreamError::Idle)))));
    assert!(matches!(poll!(events.next()), Poll::Ready(None)));
}

#[tokio::test(start_paused = true)]
async fn an_empty_read_does_not_reset_idle() {
    let chunks = stream::once(async {
        tokio::time::sleep(Duration::from_secs(40)).await;
        Ok(&b""[..])
    })
    .chain(stream::pending());
    let mut events = read_stream(chunks);
    let next = events.next();
    pin_mut!(next);
    assert!(poll!(&mut next).is_pending());
    advance(Duration::from_secs(40)).await;
    assert!(poll!(&mut next).is_pending());
    advance(Duration::from_secs(5)).await;
    assert!(matches!(poll!(next), Poll::Ready(Some(Err(ChangeStreamError::Idle)))));
}

#[tokio::test(start_paused = true)]
async fn keep_alives_reset_idle_but_the_stream_expires_at_24_hours() {
    let spacing = Duration::from_secs(40);
    let chunks = stream::unfold(Instant::now() + spacing, move |next| async move {
        sleep_until(next).await;
        Some((Ok(&b": keep-alive\n\n"[..]), next + spacing))
    });
    let mut events = read_stream(chunks);
    let wall_start = std::time::Instant::now();
    let next = events.next();
    pin_mut!(next);
    for _ in 0..LIFETIME.as_secs() / spacing.as_secs() {
        assert!(wall_start.elapsed() < Duration::from_secs(5));
        assert!(poll!(&mut next).is_pending());
        advance(spacing).await;
    }
    assert!(matches!(poll!(next), Poll::Ready(Some(Err(ChangeStreamError::Lifetime)))));
    assert!(matches!(poll!(events.next()), Poll::Ready(None)));
}
