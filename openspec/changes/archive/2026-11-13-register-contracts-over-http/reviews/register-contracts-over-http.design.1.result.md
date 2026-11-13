Verdict: not yet

Blocking findings:

1. Unbounded schemas can make the registry unreadable. In design.md, “Contract validation” explicitly accepts arbitrary core schema depth while “Compatibility and scope” excludes backend changes. SQLite writes contracts with Value::to_string(), then reads them with serde_json::from_str (crates/cicada/src/store/sqlite/rows.rs, parse_json/type_def), whose nesting limit remains enabled. A structurally valid schema nested 200 levels can therefore be acknowledged but fail subsequent type reads as corrupt; resolution reads the entire registry, affecting unrelated types too. Set a schema-specific storage/response-safe depth bound (it need not be 64), or explicitly expand scope to support deep schemas throughout storage, hashing and client decoding. Add an accepted-boundary registration/read/retry scenario on SQL, not just a core acceptance test above 64.

2. The prescribed resolver conversion can return the wrong stored descriptions. In design.md, “Register without hosting” requires TypeResolver::registered and promises the accepted stored contract; “Compatibility and scope” permits deletion and re-registration. The resolver caches by description-insensitive hash (runtime/resolver.rs), and remove_type does not invalidate it. Register description A, delete the unused registration, then register description B: the store holds B but RegisteredType.contract and TypeView return cached A. Specify conversion/cache behavior that respects the returned TypeDef, including changes made through another ActorSystem, and add a delete/re-register/read scenario. Local-only invalidation is insufficient for shared-store peers.

Notes: Scope otherwise fits one behavior and four implementation tasks. OpenSpec strict validation passed; adjacent proposals do not introduce a conflict.

Owner questions: None.

Not checked: No build, runtime tests, test suite, database execution or HTTP experiments were run; findings follow the written design and source paths.
