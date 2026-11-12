//! A refused request costs memory in proportion to its size (http-schema-check). A counting allocator measures the most
//! bytes held at once: during the check of a value alone (through the sample check of a state, which copies nothing),
//! and during a request that the door refuses at the check, above the same request refused before the check. A test
//! binary of its own, with one test, so that no other test allocates meanwhile.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, header};
use cicada::prelude::*;
use cicada::{ActorSystem, Store};
use peak_alloc::PeakAlloc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tower::ServiceExt;

#[global_allocator]
static ALLOCATOR: PeakAlloc = PeakAlloc;

/// An ordinary type: a list of lines and a few fields, as the validator measured it.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Order {
    id: String,
    lines: Vec<Line>,
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Line {
    sku: String,
    quantity: u32,
}

/// A list that refers to itself through an `Option`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Node {
    items: Vec<i32>,
    next: Option<Box<Node>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Orders {
    id: String,
}

impl ActorRef for Orders {
    const TYPE: &'static str = "orders";
    type Message = OrderMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum OrderMsg {
    Place { order: Order },
}

struct OrderActor;

impl Actor for OrderActor {
    type Ref = Orders;
    type State = Order;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: OrderMsg) -> HandlerResult {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Nodes {
    id: String,
}

impl ActorRef for Nodes {
    const TYPE: &'static str = "nodes";
    type Message = NodeMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum NodeMsg {
    Grow { node: Node },
}

struct NodeActor;

impl Actor for NodeActor {
    type Ref = Nodes;
    type State = Node;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: NodeMsg) -> HandlerResult {
        Ok(())
    }
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Map<_, _>>())
}

/// An order whose lines are 520,000 numbers where objects are wanted: a body of about 1 MiB.
fn order() -> Value {
    object(vec![("id", json!("o-1")), ("lines", Value::Array(vec![json!(0); 520_000])), ("note", Value::Null)])
}

/// 100 levels of a list that refers to itself, each with 2,000 texts where numbers are wanted.
fn node() -> Value {
    let level = |next: Value| object(vec![("items", Value::Array(vec![json!("x"); 2_000])), ("next", next)]);
    let mut node = level(Value::Null);
    for _ in 1..100 {
        node = level(node);
    }
    node
}

/// The most bytes held at once while `work` runs, above what was held before.
async fn most<T>(work: impl Future<Output = T>) -> (usize, T) {
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let result = work.await;
    (ALLOCATOR.peak_usage() - before, result)
}

/// Sends `body` and returns the status and the detail.
async fn call(app: &Router, method: &str, uri: &str, body: String) -> (u16, String) {
    let request = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json");
    let response = app.clone().oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let problem: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, problem["detail"].as_str().unwrap_or_default().to_owned())
}

/// Scenario: A request with many mismatches. The check holds at most a few times the body, alone and within a request,
/// for a payload and a state; with the first validator it held about 470 times the body.
#[tokio::test]
async fn a_refused_request_costs_memory_in_proportion_to_its_size() {
    std::panic::set_hook(Box::new(|_| {})); // the sample checks refuse by panicking
    // The check of a value alone: the sample check of a state copies nothing before the check.
    for (what, value) in [("an order", order()), ("a deep list", node())] {
        let size = value.to_string().len();
        let (held, refused) = most(async {
            let sample = std::panic::AssertUnwindSafe(|| {
                if what == "an order" {
                    cicada_axum::assert_state_sample::<Order>(&value)
                } else {
                    cicada_axum::assert_state_sample::<Node>(&value)
                }
            });
            std::panic::catch_unwind(sample).is_err()
        })
        .await;
        assert!(refused, "{what} is refused");
        println!("the check alone of {what} of {size} bytes held at most {held} bytes");
        assert!(held < 4 * size, "the check alone of {what} held {held} bytes for {size}");
    }

    // A request: the same body refused before the check (an unknown message type, a type that is not registered) and
    // at the check. What the check adds is the difference.
    let system = ActorSystem::builder(Store::memory()).host(OrderActor).host(NodeActor).build();
    system.initialize().await.unwrap(); // checks and decodes at the door, runs no turn
    let app = cicada_axum::actor_api(system);
    let empty_order = object(vec![("id", json!("o-1")), ("lines", json!([])), ("note", Value::Null)]);
    let created = |state: Value| object(vec![("state", state)]).to_string();
    assert_eq!(call(&app, "PUT", "/actors/orders/o", created(empty_order)).await.0, 201);
    assert_eq!(call(&app, "PUT", "/actors/nodes/n", created(json!({"items": []}))).await.0, 201);
    let send = |message_type: &str, field: &str, value: Value| {
        object(vec![("type", json!(message_type)), ("payload", object(vec![(field, value)]))]).to_string()
    };
    let cases = [
        (
            "an order as a payload",
            "POST",
            "/actors/orders/o/messages",
            "/actors/orders/o/messages",
            send("Nope", "order", order()),
            send("Place", "order", order()),
        ),
        ("an order as a state", "PUT", "/actors/ghosts/p", "/actors/orders/p", created(order()), created(order())),
        (
            "a deep list as a payload",
            "POST",
            "/actors/nodes/n/messages",
            "/actors/nodes/n/messages",
            send("Nope", "node", node()),
            send("Grow", "node", node()),
        ),
        ("a deep list as a state", "PUT", "/actors/ghosts/m", "/actors/nodes/m", created(node()), created(node())),
    ];
    for (what, method, before_uri, uri, before_body, body) in cases {
        let size = body.len();
        let (before, (status, _)) = most(call(&app, method, before_uri, before_body)).await;
        assert!(status == 422 || status == 404, "{what} is refused before the check: {status}");
        let (held, (status, detail)) = most(call(&app, method, uri, body)).await;
        assert_eq!(status, 422, "{what}: {detail}");
        assert!(detail.contains("does not match the schema"), "{what}: {detail}");
        let added = held.saturating_sub(before);
        println!("{what}, a body of {size} bytes: refused before the check {before}, at the check {held} bytes");
        assert!(added < 4 * size, "the check of {what} added {added} bytes for a body of {size}");
    }
}
