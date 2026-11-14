# Install Cicada

Add Cicada as a Git dependency. The repository is private and the crates are not published to a registry.
Choose a release from the [changelog](../../CHANGELOG.md); keep every Cicada crate on the same tag and URL.

## Choose the crates

For an app binary backed by SQLite:

```toml
[dependencies]
cicada = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.17.2" }
cicada-app = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.17.2", features = ["sqlite"] }
serde = { version = "1", features = ["derive"] }
schemars = "1"
```

| Need | Dependency or feature |
| --- | --- |
| Open a store directly | Enable `sqlite` or `postgres` on `cicada` |
| Open it through `App` | Enable the store feature on `cicada-app`; it enables the core feature too |
| Use a typed HTTP client | Add `cicada-client` from the same tag |
| Mount the API | Add `cicada-axum` from the same tag |
| Use ready-made actors | Add `cicada-actors` from the same tag |
| Test actors | Add `cicada-test` as a dev dependency and `tokio` with `test-util` |
| Use the terminal UI | Enable `tui` on `cicada-app` |

No store or terminal feature is enabled by default. An HTTP worker needs no store feature.
In a workspace, place shared entries under `[workspace.dependencies]` and inherit them from each package.

## Authenticate Cargo

Configure Cargo to use Git's credentials in `~/.cargo/config.toml`:

```toml
[net]
git-fetch-with-cli = true
```

On a developer machine, run `gh auth login`, then `gh auth setup-git`.

In CI, use a token with read access to this repository; the default `GITHUB_TOKEN` only reaches its own repository.
Keep credentials out of manifests and lockfiles. Supply the credential to Git through your CI's secret mechanism.
For Docker builds, use a BuildKit secret in the build step; never bake the token into an image layer.

## Upgrade

Change all Cicada tags together, then follow the [upgrade procedure](upgrading.md).
While the version is 0.x, breaking changes raise the minor version; other changes raise the patch.
Public Rust APIs, HTTP formats, schema versions, contract hashes and guarantees are compatibility boundaries.
Published tags never move.

Next: [build your first actor](../guide/core.md#define-an-actor-type).
