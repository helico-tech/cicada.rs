//! The Messages page's model against an in-process server: the paged list of pending messages, each once, and the
//! pane filled from the selected row. Real sockets and the real clock, as in the parent module; every wait has a bound.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use cicada::prelude::*;
use cicada::store::MessageId;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::timeout;

use super::{Counter, Fake, Rule, Server, Work, actors, no_rule, serve};
use crate::messages::{MessagePane, Messages, MessagesView};
use crate::model::Limits;
use crate::pager::Paging;

/// The longest a test waits for one change.
const WAIT: Duration = Duration::from_secs(10);

pub(super) fn model(server: &Server, limits: Limits) -> Messages {
    let messages = Messages::new(tokio::runtime::Handle::current(), limits);
    messages.use_client(Some(server.client.clone()));
    messages
}

pub(super) async fn list_until(
    list: &mut watch::Receiver<MessagesView>,
    done: impl FnMut(&MessagesView) -> bool,
) -> MessagesView {
    timeout(WAIT, list.wait_for(done)).await.expect("the list changes in time").unwrap().clone()
}

pub(super) async fn pane_until(
    pane: &mut watch::Receiver<MessagePane>,
    done: impl FnMut(&MessagePane) -> bool,
) -> MessagePane {
    timeout(WAIT, pane.wait_for(done)).await.expect("the pane changes in time").unwrap().clone()
}

/// A served envelope.
pub(super) fn envelope(id: &str, to: &str, attempt: u32) -> Value {
    json!({"id": id, "to": to, "from": "bag/sender", "type": "Ok", "payload": {"n": attempt, "name": "zoë"},
           "deliverAt": "2026-11-27T10:00:00.000Z", "correlationId": "order-7", "headers": {"trace": "t-1"},
           "seq": 1, "attempt": attempt, "failures": 1})
}

pub(super) fn id(id: &str) -> MessageId {
    MessageId::try_from(id.to_owned()).unwrap()
}

fn page(items: &[Value], next: Option<&str>) -> Fake {
    Fake::Answer(StatusCode::OK, json!({"items": items, "next": next}))
}

fn ids(view: &MessagesView) -> Vec<String> {
    view.rows.iter().map(|row| row.id.to_string()).collect()
}

fn served(server: &mut Server, prefix: &str) -> Vec<String> {
    std::iter::from_fn(|| server.events.try_recv().ok()).filter(|e| e.starts_with(prefix)).collect()
}

/// Scenario: A pending message, paged. 150 messages are read in two pages by the server's cursor, with their id,
/// target, type, due time and attempt; the rows of the first page are shared with the later view, not copied.
#[tokio::test]
async fn one_hundred_fifty_messages_are_read_in_two_pages() {
    let actors = actors().await;
    let mut sent = Vec::new();
    for n in 0..150 {
        let counter = Counter { id: format!("{n:03}") };
        actors.create(&counter, &json!({})).await;
        sent.push(actors.send(&counter, Work::Ok).await);
    }
    let mut server = serve(&actors, no_rule(), Duration::ZERO).await;
    let model = model(&server, Limits { debounce: Duration::from_millis(10), ..Limits::DEFAULT });
    let mut list = model.list();
    model.search("counter/");
    let first = list_until(&mut list, |l| l.paging == Paging::Idle && !l.rows.is_empty()).await;
    assert_eq!((first.rows.len(), first.more, first.prefix.as_str()), (100, true, "counter/"));
    assert!(!first.wants_more(50) && first.wants_more(80));
    model.more();
    model.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 150).await;
    assert!(!view.more);
    assert!(Arc::ptr_eq(&first.rows[0], &view.rows[0]), "a later view shares the rows read before");
    assert_eq!(ids(&view), sent.iter().map(|m| m.id.to_string()).collect::<Vec<_>>());
    let row = &view.rows[0];
    assert_eq!((row.to.as_str(), row.message_type.as_str(), row.attempt), ("counter/000", "Ok", 0));
    assert_eq!(row.deliver_at, sent[0].deliver_at);
    let pages = served(&mut server, "served /messages");
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert_eq!(pages[0], "served /messages?prefix=counter%2F&limit=100");
    assert!(pages[1].starts_with("served /messages?prefix=counter%2F&cursor="), "{pages:?}");
}

/// Scenario: Duplicates across pages. A message retried between pages is on two of them: the first row is kept and
/// the later one dropped. A page of only duplicates leaves the rows as they were, yet its cursor is used: the next
/// page is asked for at once, without the view asking again.
#[tokio::test]
async fn a_message_on_two_pages_is_shown_once_and_a_page_of_duplicates_still_advances() {
    let actors = actors().await;
    let pages = [
        ("/messages?limit=100", page(&[envelope("m1", "bag/a", 1), envelope("m2", "bag/b", 1)], Some("c1"))),
        ("/messages?cursor=c1&limit=100", page(&[envelope("m2", "bag/b", 2), envelope("m3", "bag/c", 1)], Some("c2"))),
        ("/messages?cursor=c2&limit=100", page(&[envelope("m3", "bag/c", 2), envelope("m3", "bag/c", 3)], Some("c3"))),
        ("/messages?cursor=c3&limit=100", page(&[envelope("m4", "bag/d", 1)], None)),
    ];
    let rule: Rule =
        Arc::new(move |target| pages.iter().find(|(path, _)| *path == target).map(|(_, fake)| fake.clone()));
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits::DEFAULT);
    let mut list = model.list();
    model.show();
    list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    model.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 3).await;
    assert_eq!(ids(&view), ["m1", "m2", "m3"]);
    assert_eq!(view.rows[1].attempt, 1, "the first row of m2 is kept");
    assert!(view.more && view.wants_more(2));

    model.more();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && !l.more).await;
    assert_eq!(ids(&view), ["m1", "m2", "m3", "m4"]);
    assert_eq!(view.rows[2].attempt, 1, "the first row of m3 is kept");
    let asked = served(&mut server, "served /messages");
    assert_eq!(
        asked,
        [
            "served /messages?limit=100",
            "served /messages?cursor=c1&limit=100",
            "served /messages?cursor=c2&limit=100",
            "served /messages?cursor=c3&limit=100",
        ]
    );
}

/// Scenario: A pending message. The pane is filled from the selected row, with no read of its own; its payload is
/// formatted only then, and a formatting of an earlier selection is never shown. A search that leaves out the
/// message's target unselects it, and Refresh clears the selection.
#[tokio::test]
async fn the_pane_comes_from_the_row_and_formats_the_payload_only_on_selection() {
    let actors = actors().await;
    let rows = [envelope("m1", "bag/a", 1), envelope("m2", "bag/b", 2)];
    let first = page(&rows, None);
    let rule: Rule = Arc::new(move |target| target.starts_with("/messages").then(|| first.clone()));
    let mut server = serve(&actors, rule, Duration::ZERO).await;
    let model = model(&server, Limits { debounce: Duration::from_millis(10), ..Limits::DEFAULT });
    let (mut list, mut pane) = (model.list(), model.pane());
    model.show();
    let view = list_until(&mut list, |l| l.paging == Paging::Idle && l.rows.len() == 2).await;
    assert_eq!(model.formatted(), 0, "listing formats no payload");
    assert_eq!(pane.borrow().message, None);
    served(&mut server, "");

    let (m1, m2) = (id("m1"), id("m2"));
    model.select(Some(&m1));
    model.select(Some(&m2));
    let shown = pane_until(&mut pane, |p| p.payload.is_some()).await;
    let message = shown.message.as_ref().expect("a message is selected");
    assert!(Arc::ptr_eq(message, &view.rows[1]), "the pane shares the row");
    assert_eq!((message.id.as_ref(), message.to.as_str(), message.attempt, message.failures), ("m2", "bag/b", 2, 1));
    assert_eq!(message.from.as_ref().map(ToString::to_string).as_deref(), Some("bag/sender"));
    assert_eq!(message.correlation_id.as_deref(), Some("order-7"));
    assert_eq!(message.headers.get("trace").map(String::as_str), Some("t-1"));
    assert_eq!(shown.payload.as_ref().unwrap().text, "{\n  \"n\": 2,\n  \"name\": \"zoë\"\n}");
    for task in model.take_aborted() {
        let ended = timeout(WAIT, task).await.expect("an aborted formatting ends");
        assert!(ended.as_ref().is_err_and(|e| e.is_cancelled()) || ended.is_ok(), "{ended:?}");
    }
    assert_eq!(pane.borrow().message.as_ref().map(|m| m.id.clone()), Some(m2.clone()));
    assert!(served(&mut server, "").is_empty(), "the pane reads nothing: {:?}", served(&mut server, ""));

    model.search("bag/");
    assert_eq!(model.selected(), Some(m2.clone()), "a prefix that holds the target keeps it");
    model.search("bag/a");
    assert_eq!((model.selected(), pane.borrow().message.clone()), (None, None));
    list_until(&mut list, |l| l.paging == Paging::Idle && l.prefix == "bag/a").await;
    model.select(Some(&m1));
    pane_until(&mut pane, |p| p.payload.is_some()).await;
    model.refresh();
    assert_eq!((model.selected(), pane.borrow().message.clone()), (None, None));
    model.select(Some(&id("not-a-row")));
    assert_eq!((model.selected(), pane.borrow().message.clone()), (None, None));
}
