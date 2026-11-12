//! A refusal by the schema costs time and memory in proportion to the size of the request (http-schema-check). A
//! counting allocator measures the most bytes held at once while the door refuses a request at the check, after which
//! nothing else runs, above the same request refused before the check, on a node that hosts the types and on one that
//! does not; each refusal is timed. A test binary of its own, with one test, so that no other test allocates meanwhile.

use std::time::{Duration, Instant};

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

/// The same lines in a part that may be left out: an `anyOf` in the schema.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Basket {
    id: String,
    batch: Option<Batch>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Batch {
    lines: Vec<Line>,
}

/// A list that refers to itself through an `Option`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Node {
    items: Vec<i32>,
    next: Option<Box<Node>>,
}

/// An untagged enum whose variants share a field: the decoder of serde tries every variant at every level.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
enum Tree {
    Leaf(i64),
    Pair { l: Box<Tree>, r: Box<Tree> },
    Note { l: Box<Tree>, note: Option<String> },
}

/// Operators whose variants share the names of their fields: a `oneOf` at every level.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op")]
enum Expr {
    Lit { v: i64 },
    Add { l: Box<Expr>, r: Box<Expr> },
    Mul { l: Box<Expr>, r: Box<Expr> },
    Sub { l: Box<Expr>, r: Box<Expr> },
}

/// A test type whose state is `S` and whose message `Put` carries a `v` of type `V`.
macro_rules! test_type {
    ($ref:ident, $msg:ident, $actor:ident, $name:literal, $state:ty, $value:ty) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct $ref {
            id: String,
        }

        impl ActorRef for $ref {
            const TYPE: &'static str = $name;
            type Message = $msg;
        }

        #[derive(Debug, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "type")]
        enum $msg {
            Put { v: $value },
        }

        struct $actor;

        impl Actor for $actor {
            type Ref = $ref;
            type State = $state;

            async fn handle(&self, _turn: &mut Turn<Self>, _message: $msg) -> HandlerResult {
                Ok(())
            }
        }
    };
}

test_type!(Orders, OrderMsg, OrderActor, "orders", Order, Order);
test_type!(Baskets, BasketMsg, BasketActor, "baskets", Basket, Basket);
test_type!(Nodes, NodeMsg, NodeActor, "nodes", Node, Node);
test_type!(Calcs, CalcMsg, CalcActor, "calcs", Expr, Expr);
test_type!(Trees, TreeMsg, TreeActor, "trees", Tree, Tree);

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Map<_, _>>())
}

/// An order whose lines are 520,000 numbers where objects are wanted: a body of about 1 MiB.
fn order() -> Value {
    object(vec![("id", json!("o-1")), ("lines", Value::Array(vec![json!(0); 520_000])), ("note", Value::Null)])
}

/// A basket whose optional batch has 520,000 numbers where lines are wanted.
fn basket() -> Value {
    object(vec![("id", json!("b-1")), ("batch", object(vec![("lines", Value::Array(vec![json!(0); 520_000]))]))])
}

/// 100 levels of a list that refers to itself, each with 2,000 items: wrong ones in every level, or below the first
/// level only (in the optional part).
fn node(first_level_matches: bool) -> Value {
    let level = |item: Value, next: Value| object(vec![("items", Value::Array(vec![item; 2_000])), ("next", next)]);
    let mut node = level(json!("x"), Value::Null);
    for depth in 1..100 {
        let item = if first_level_matches && depth == 99 { json!(1) } else { json!("x") };
        node = level(item, node);
    }
    node
}

/// A tree `depth` deep with one wrong leaf.
fn tree(depth: usize) -> Value {
    let mut value = json!("x");
    for _ in 0..depth {
        value = object(vec![("l", value), ("r", json!(1))]);
    }
    value
}

/// An expression `depth` deep with one wrong leaf.
fn expr(depth: usize) -> Value {
    let lit = |v: Value| object(vec![("op", json!("Lit")), ("v", v)]);
    let mut value = lit(json!("x"));
    for i in 0..depth {
        value = object(vec![("op", json!(["Add", "Mul", "Sub"][i % 3])), ("l", value), ("r", lit(json!(1)))]);
    }
    value
}

/// The most bytes held at once while `work` runs, above what was held before, and how long it took.
async fn measure<T>(work: impl Future<Output = T>) -> (usize, Duration, T) {
    let before = ALLOCATOR.current_usage();
    ALLOCATOR.reset_peak_usage();
    let start = Instant::now();
    let result = work.await;
    (ALLOCATOR.peak_usage() - before, start.elapsed(), result)
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

/// Scenarios: A request with many mismatches; A small nested value with one mistake. The check adds at most a few
/// times the body (and what compiling the schema takes) and answers within seconds, on both nodes, for a payload and a
/// state. Asking the validator for a place held hundreds of MiB for these and took minutes for the expression, and the
/// decoder of a hosting node took seconds for the tree, doubling per level.
#[tokio::test]
async fn a_refused_request_costs_time_and_memory_in_proportion_to_its_size() {
    let store = Store::memory();
    let host = ActorSystem::builder(store.clone())
        .host(OrderActor)
        .host(BasketActor)
        .host(NodeActor)
        .host(CalcActor)
        .host(TreeActor)
        .build();
    host.initialize().await.unwrap(); // checks and decodes at the door, runs no turn
    let other = ActorSystem::builder(store).build();
    let nodes = [("the hosting node", cicada_axum::actor_api(host)), ("the other node", cicada_axum::actor_api(other))];
    let created = |state: Value| object(vec![("state", state)]).to_string();
    let small = [
        ("orders", object(vec![("id", json!("o")), ("lines", json!([])), ("note", Value::Null)])),
        ("baskets", object(vec![("id", json!("b")), ("batch", Value::Null)])),
        ("nodes", json!({"items": []})),
        ("calcs", json!({"op": "Lit", "v": 1})),
        ("trees", json!(1)),
    ];
    for (name, state) in small {
        assert_eq!(call(&nodes[0].1, "PUT", &format!("/actors/{name}/a"), created(state)).await.0, 201);
    }
    let send = |message_type: &str, value: Value| {
        object(vec![("type", json!(message_type)), ("payload", object(vec![("v", value)]))]).to_string()
    };
    type Case = (&'static str, &'static str, fn() -> Value);
    let cases: [Case; 6] = [
        ("an order of 520,000 wrong lines", "orders", order),
        ("an optional batch of 520,000 wrong lines", "baskets", basket),
        ("100 levels of a list, wrong in every level", "nodes", || node(false)),
        ("100 levels of a list, wrong below the first level", "nodes", || node(true)),
        ("an expression 30 deep with one wrong leaf", "calcs", || expr(30)),
        ("an untagged tree 24 deep with one wrong leaf", "trees", || tree(24)),
    ];
    for (what, name, value) in cases {
        for (node, app) in &nodes {
            let kinds = [
                (
                    "a payload",
                    "POST",
                    format!("/actors/{name}/a/messages"),
                    send("Nope", value()),
                    send("Put", value()),
                ),
                ("a state", "PUT", format!("/actors/ghosts/{name}"), created(value()), created(value())),
            ];
            for (kind, method, before_uri, before_body, body) in kinds {
                let uri = if kind == "a state" { format!("/actors/{name}/new") } else { before_uri.clone() };
                let size = body.len();
                let (before, _, (status, _)) = measure(call(app, method, &before_uri, before_body)).await;
                assert!(status == 422 || status == 404, "{what} as {kind} is refused before the check: {status}");
                let (held, took, (status, detail)) = measure(call(app, method, &uri, body)).await;
                let case = format!("{what} as {kind} on {node}");
                assert_eq!(status, 422, "{case}: {detail:.300}");
                assert!(detail.contains("does not match the registered schema; GET /types/"), "{case}: {detail:.300}");
                let added = held.saturating_sub(before);
                println!("{case}, a body of {size} bytes: {took:?}; held {before} before the check, {held} at it");
                assert!(added < 4 * size + (1 << 20), "{case}: the refusal added {added} bytes for a body of {size}");
                assert!(took < Duration::from_secs(10), "{case}: the refusal took {took:?}");
            }
        }
    }
}
