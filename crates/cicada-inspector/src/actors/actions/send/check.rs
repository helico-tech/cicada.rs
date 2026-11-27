//! The local check of a payload, by the settings of the server's door in `cicada-axum`'s `src/schema_check.rs`: the
//! same `jsonschema`, draft 2020-12 whatever `$schema` says, no `format` asserted, and nothing loaded from outside the
//! schema. As the door, it refuses a payload nested deeper than [`MAX_DEPTH`] before the schema, and asks only
//! `is_valid`, which builds no error. The server's check stays the authority.
//!
//! One thread per sheet, with the door's stack of 64 MiB, runs one check at a time: it takes the newest job, skipping
//! older ones still queued, and the sheet drops a result for an older payload or message type.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Weak};

use cicada::runtime::limits::MAX_DEPTH;
use jsonschema::{Draft, Retrieve, Uri, Validator};
use serde_json::Value;

use crate::json::{self, Json};

/// The longest payload text checked: 1 MiB.
pub(crate) const MAX_TEXT: usize = 1 << 20;
/// The door's stack for a check (its `CHECK_STACK`).
const STACK: usize = 64 << 20;

/// What the check says of a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Not checked yet: the payload or the message type changed.
    Checking,
    /// The payload matches the schema.
    Passes,
    /// The payload is JSON within the limits, and the schema cannot be compiled: the server does not check it either.
    Unchecked,
    /// Refused: why.
    Refused(String),
}

impl Verdict {
    /// The refusal of a payload that does not match: the schema is shown beside it, and no place is named.
    pub const MISMATCH: &str = "does not match the schema";
    /// The note on a message type whose schema cannot be compiled.
    pub const UNCHECKED: &str = "the server does not check this type's payloads";
}

/// A payload to check against the schema of a chosen message type.
pub(super) struct Job {
    pub sheet: u64,
    /// Grows each time a message type is chosen; the schema is compiled once per value.
    pub choice: u64,
    /// Grows at every edit and choice.
    pub edit: u64,
    pub schema: Arc<Value>,
    pub text: Arc<str>,
}

/// What the check thread lands.
pub(super) enum Landed {
    /// The chosen schema, formatted, and whether it compiles.
    Schema { sheet: u64, choice: u64, json: Arc<Json>, compiles: bool },
    /// The verdict on a payload, with its parsed value when it may be sent.
    Verdict { sheet: u64, choice: u64, edit: u64, verdict: Verdict, value: Option<Arc<Value>> },
}

/// The sheet's check thread. It ends when this is dropped, after the check it runs, if any.
pub(super) struct Checker {
    jobs: Sender<Job>,
    #[cfg(test)]
    pub holds: Arc<std::sync::Mutex<std::collections::VecDeque<Hold>>>,
}

#[cfg(test)]
pub(crate) type Hold = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

/// Where the thread lands its results: the sheet's model, which it must not keep alive.
pub(super) trait Land: Send + Sync + 'static {
    fn land(&self, landed: Landed);
}

impl Checker {
    /// Starts the thread; `None` when it cannot be started.
    pub fn start<L: Land>(land: Weak<L>) -> Option<Self> {
        let (jobs, queue) = channel();
        #[cfg(test)]
        let holds = Arc::<std::sync::Mutex<std::collections::VecDeque<Hold>>>::default();
        #[cfg(test)]
        let held = holds.clone();
        let thread = std::thread::Builder::new().name("cicada-inspector-check".into()).stack_size(STACK);
        thread
            .spawn(move || {
                // A test holds a check until it releases it, to order results.
                #[cfg(test)]
                let before = move || {
                    let hold = held.lock().unwrap().pop_front();
                    if let Some((arrived, release)) = hold {
                        let _ = arrived.send(());
                        let _ = release.recv_timeout(std::time::Duration::from_secs(10));
                    }
                };
                #[cfg(not(test))]
                let before = || {};
                run(queue, land, before)
            })
            .ok()?;
        Some(Self {
            jobs,
            #[cfg(test)]
            holds,
        })
    }

    pub fn post(&self, job: Job) {
        // The thread only ends when this sender is dropped, or a check panicked; then nothing lands.
        let _ = self.jobs.send(job);
    }
}

fn run<L: Land>(queue: Receiver<Job>, land: Weak<L>, before: impl Fn()) {
    let mut compiled: Option<(u64, Result<Validator, String>)> = None;
    while let Ok(mut job) = queue.recv() {
        while let Ok(newer) = queue.try_recv() {
            job = newer;
        }
        before();
        let validator = match &compiled {
            Some((choice, validator)) if *choice == job.choice => validator,
            _ => {
                let validator = validator(&job.schema);
                let json = Arc::new(json::format(&job.schema));
                let compiles = validator.is_ok();
                let Some(land) = land.upgrade() else { return };
                land.land(Landed::Schema { sheet: job.sheet, choice: job.choice, json, compiles });
                &compiled.insert((job.choice, validator)).1
            }
        };
        let (verdict, value) = check(validator, &job.text);
        let Some(land) = land.upgrade() else { return };
        land.land(Landed::Verdict { sheet: job.sheet, choice: job.choice, edit: job.edit, verdict, value });
    }
}

/// A retriever that refuses every address, as the door's `NoRetriever`.
struct NoRetriever;

impl Retrieve for NoRetriever {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("{uri} is outside the schema, and the check loads nothing").into())
    }
}

/// The door's validator settings (`validator` in `cicada-axum`'s `src/schema_check.rs`).
pub(crate) fn validator(schema: &Value) -> Result<Validator, String> {
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(false)
        .with_retriever(NoRetriever)
        .build(schema)
        .map_err(|e| e.to_string())
}

/// Size, parse and depth, then the schema; the parsed value when it may be sent.
pub(crate) fn check(validator: &Result<Validator, String>, text: &str) -> (Verdict, Option<Arc<Value>>) {
    if text.len() > MAX_TEXT {
        return (Verdict::Refused("larger than 1 MiB".into()), None);
    }
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => return (Verdict::Refused(format!("does not parse: {error}")), None),
    };
    if too_deep(&value, 0) {
        return (Verdict::Refused(format!("nested deeper than {MAX_DEPTH} levels")), None);
    }
    match validator {
        Ok(validator) if validator.is_valid(&value) => (Verdict::Passes, Some(Arc::new(value))),
        Ok(_) => (Verdict::Refused(Verdict::MISMATCH.into()), None),
        Err(_) => (Verdict::Unchecked, Some(Arc::new(value))),
    }
}

/// True when `value`, inside `depth` arrays and objects, holds a container inside [`MAX_DEPTH`] others, counted as the
/// server's `limits::walk` counts: a scalar is 0 deep, each array or object adds one. The parser stops at 128 levels.
fn too_deep(value: &Value, depth: usize) -> bool {
    match value {
        Value::Object(object) => depth >= MAX_DEPTH || object.values().any(|item| too_deep(item, depth + 1)),
        Value::Array(array) => depth >= MAX_DEPTH || array.iter().any(|item| too_deep(item, depth + 1)),
        _ => false,
    }
}
