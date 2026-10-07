//! Helpers shared by the example tests.

use cicada_test::TestActors;

/// The instance URIs starting with `prefix` (`GET /actors?prefix=`).
pub async fn list(test: &TestActors, prefix: &str) -> Vec<String> {
    test.list(prefix).await.iter().map(|a| a.as_str().to_owned()).collect()
}
