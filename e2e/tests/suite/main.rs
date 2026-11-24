//! The E2E and fault suite: one module per test class, run in a fixed order
//! against one stack by `cicada_e2e::run_suite`. Runs only with `CICADA_E2E=1`; see e2e/README.md.

mod cluster_api;
mod cluster_examples;
mod cluster_restart;
mod container_docker;
mod node_fault;
mod ordering;
mod outcomes_under_fault;
mod sample_executor;
mod sqlite_node;
mod store_outage;

use std::time::Duration;

/// One second.
const SECOND: Duration = Duration::from_secs(1);

/// `n` seconds.
const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// `n` milliseconds.
const fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn e2e() {
    cicada_e2e::run_suite(vec![
        cluster_api::class(),          // @Order(1)
        cluster_examples::class(),     // @Order(2)
        ordering::class(),             // @Order(3)
        node_fault::class(),           // @Order(4)
        outcomes_under_fault::class(), // @Order(5)
        store_outage::class(),         // @Order(6)
        cluster_restart::class(),      // @Order(7)
        sqlite_node::class(),          // @Order(8)
        sample_executor::class(),      // @Order(9)
        container_docker::class(),     // @Order(10)
    ]);
}
