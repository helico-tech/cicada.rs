//! Helpers of the runtime tests that need only the public API. The runtime tests that need the store trait, with
//! their own helpers, are in `crates/cicada/src/tests/runtime`.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use cicada::store::{Delivery, TurnOutcome};
use cicada::time;

/// `Some(v)` as `v`, `None` as `null`.
pub fn opt<T: std::fmt::Display>(value: &Option<T>) -> String {
    value.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "null".to_owned())
}

/// A shared event log.
#[derive(Debug, Clone, Default)]
pub struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub fn push(&self, event: impl Into<String>) {
        self.0.lock().unwrap().push(event.into());
    }

    pub fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    pub fn last(&self) -> String {
        self.0.lock().unwrap().last().cloned().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// A turn outcome as text: `Done`, `Failed(error=…)`, `Rescheduled(delivery=…, error=…)`.
pub fn outcome_text(outcome: &TurnOutcome) -> String {
    match outcome {
        TurnOutcome::Done => "Done".to_owned(),
        TurnOutcome::Failed(e) => format!("Failed(error={e})"),
        TurnOutcome::Cancelled => "Cancelled".to_owned(),
        TurnOutcome::Rescheduled { delivery, error } => {
            let delivery = match delivery {
                Delivery::Now => "Now".to_owned(),
                Delivery::At(t) => format!("At(instant={})", time::format_instant(*t)),
                Delivery::After(d) => format!("After(delay={d:?})"),
            };
            format!("Rescheduled(delivery={delivery}, error={error})")
        }
    }
}
