# Install Cicada

Add Cicada as a Git dependency. The repository is private and the crates are not published to a registry.
Choose a release from the [changelog](../../CHANGELOG.md); keep every Cicada crate on the same tag and URL.

## Choose the crates

For an app binary backed by SQLite:

```toml
[dependencies]
cicada = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.18.12" }
cicada-app = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.18.12", features = ["sqlite"] }
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

## Install the coding-agent skill

The consumer skill finds your project's resolved Cicada source and routes the agent to that version's guides,
examples and code. It needs Python 3.11+, Cargo, rustc and Git. Discovery is locked and offline; it does not fetch,
upgrade dependencies or write a lockfile. Missing sources and multiple source groups are reported explicitly.

Use the archive attached to a published GitHub Release, starting with 0.17.2. For a **first install** into a project
without `.claude/skills/cicada`:

```sh
staging=$(mktemp -d)
gh release download v0.18.12 --repo helico-tech/cicada.rs --pattern 'cicada-skill-*.tar.gz' --dir "$staging"
tar -xzf "$staging/cicada-skill-v0.18.12.tar.gz" -C "$staging"
# Inspect the skill and helper before copying them into your agent's project skill folder.
mkdir -p .claude/skills
cp -R "$staging/cicada" .claude/skills/
```

The directory shown is for Claude Code; use your agent's project skill directory. From the consumer project, the
installed helper can report the matching sources:

```sh
python3 .claude/skills/cicada/scripts/cicada_source.py
```

Use the actual target/features when they differ from the host defaults. The skill's [reading map](../agents.md)
explains source selection. Equal versions can have different commits or local changes; your resolved source wins.
Without Python/toolchain, inspect Cargo.lock and verifiable local source manually rather than guessing cache paths.

### Update the installed skill

Download and unpack the new archive into a fresh staging directory as above. Nothing updates an installed copy
automatically. Compare before replacing it:

```sh
diff -r "$staging/cicada" .claude/skills/cicada
```

After reviewing the diff and preserving local changes you need, replace the folder whole so stale files do not remain:

```sh
rm -rf .claude/skills/cicada
cp -R "$staging/cicada" .claude/skills/
```

Updating the skill and updating your Cicada dependency are separate actions. Projects on releases before 0.17.2 can
use a newer router against their own older source; those older releases have no skill archive.
