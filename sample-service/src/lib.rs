//! A runnable Cicada node with the examples and the HTTP API, configured by environment variables (see
//! [`SampleConfig::from_vars`]). End-to-end tests drive it over HTTP.
//!
//! | Module | What |
//! | --- | --- |
//! | [`config`] | [`SampleConfig`], read from the environment |
//! | [`catalog`] | The hosted actor types in groups (`HOSTED_TYPES`), and the singletons created at start |
//! | [`fakes`] | Stand-ins for the external systems of the examples |
//! | [`e2e`] | Actor types for end-to-end and fault tests, and the node's [`e2e::EffectLog`] |
//! | [`app`] | [`SampleNode`]: the node, its routes and its life cycle; [`serve`] |
//! | [`telemetry`] | OTLP export of traces and metrics |

pub mod app;
pub mod catalog;
pub mod config;
pub mod e2e;
pub mod fakes;
pub mod telemetry;

pub use app::{NodeInfo, SampleNode, serve};
pub use catalog::{Catalog, GROUPS};
pub use config::{ConfigError, SampleConfig};
