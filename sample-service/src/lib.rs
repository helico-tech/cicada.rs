//! A runnable Cicada node with the examples and the HTTP API, built on `cicada-app`: `cicada-sample-service serve`,
//! configured by the flags and variables of `cicada-app` and by [`SampleConfig::from_vars`]. End-to-end tests drive it
//! over HTTP.
//!
//! | Module | What |
//! | --- | --- |
//! | [`config`] | [`SampleConfig`], the node id and the refusal of the variables before 0.13.0 |
//! | [`catalog`] | The hosted actor types, and the singletons created at start |
//! | [`fakes`] | Stand-ins for the external systems of the examples |
//! | [`e2e`] | Actor types for end-to-end and fault tests, and the node's [`e2e::EffectLog`] |
//! | [`app`] | [`Sample`]: the node's `App`, its routes and its work after the start |
//! | [`telemetry`] | OTLP export of traces and metrics |

pub mod app;
pub mod catalog;
pub mod config;
pub mod e2e;
pub mod fakes;
pub mod telemetry;

pub use app::{NodeInfo, Sample};
pub use catalog::Catalog;
pub use config::{ConfigError, OLD_VARIABLES, SampleConfig, refuse_old_variables};
