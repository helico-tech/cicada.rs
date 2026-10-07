//! Helpers shared by the example tests.

use cicada_test::TestActors;

/// The instance URIs starting with `prefix` (`GET /actors?prefix=`), first page.
pub async fn list(test: &TestActors, prefix: &str) -> Vec<String> {
    let page = test.system().list_actors(prefix, None, 100).await.expect("list failed");
    page.items.iter().map(|a| a.as_str().to_owned()).collect()
}
