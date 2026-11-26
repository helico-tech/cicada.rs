# Install Cicada

Add Cicada as a Git dependency. The repository is private and the crates are not published to a registry.
Choose a release from the [changelog](../../CHANGELOG.md); keep every Cicada crate on the same tag and URL.

## Choose the crates

For an app binary backed by SQLite:

```toml
[dependencies]
cicada = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.24.0" }
cicada-app = { git = "https://github.com/helico-tech/cicada.rs", tag = "v0.24.0", features = ["sqlite"] }
serde = { version = "1", features = ["derive"] }
schemars = "1.2.3"
```

| Need | Dependency or feature |
| --- | --- |
| Open a store directly | Enable `sqlite` or `postgres` on `cicada` |
| Open it through `App` | Enable the store feature on `cicada-app`; it enables the core feature too |
| Use a typed HTTP client | Add `cicada-client` from the same tag |
| Mount the API | Add `cicada-axum` from the same tag |
| Use ready-made actors | Add `cicada-actors` from the same tag |
| Run containers | Add `cicada-containers` from the same tag with `features = ["docker"]` for `DockerRuntime`; `features = ["testing"]` in dev dependencies for `FakeRuntime` ([guide](../guide/containers.md)) |
| Test actors | Add `cicada-test` as a dev dependency and `tokio` with `test-util` |
| Use the terminal UI | Enable `tui` on `cicada-app` |

No store, terminal, Docker or testing feature is enabled by default. An HTTP worker needs no store feature.
In a workspace, place shared entries under `[workspace.dependencies]` and inherit them from each package.

## Dependencies and lockfiles

From 0.20.6, Cicada's dependency floors are `schemars` 1.2.3 and, with `sqlite`, `sqlx` 0.9.1 and `libsqlite3-sys` 0.39.
A dependency that holds `libsqlite3-sys` below 0.39 causes a resolution error. Its upper bound comes from `sqlx`.
Run `cargo update` in your project too: Cicada's lockfile does not choose your project's dependency versions.

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

Two consumer skills use your project's resolved Cicada source:

| Skill | Use it to |
| --- | --- |
| `cicada` | Build, test, operate and upgrade using that version's guides, examples and code |
| `actor-design` | Design a new actor system with you, or review an existing design or implementation against [Design with actors](../guide/design.md) |

**Design mode** saves each step as Markdown with diagrams in `docs/design/actors.md` in your project. It waits
for confirmation before advancing, unless you explicitly delegate decisions, and resumes from the saved progress.

**Review mode** starts with a request such as “review our order actors”; name the actors or stories you want checked.
It reconstructs the system from evidence, checks it against the resolved guide and saves findings with failure traces
in `docs/design/actors-review.md`, separate from the design. One review request covers all four phases (scope and
evidence, reconstruct, check, report) without confirmation after each. An unclear request gets one question about
the mode.

Both paths are defaults; your chosen path or repository rules take precedence. A review runs no project builds,
tests or scripts and writes only its report. Its conclusions cover the evidence read, not the whole system, and are
not a proof of correctness. Neither mode implements fixes or commits its output.

Source discovery needs Python 3.11+, Cargo, rustc and Git. It is locked and offline; it does not fetch, upgrade
dependencies or write a lockfile. Missing sources and multiple source groups are reported explicitly.

**Claude Code plugin.** With Git read access to this private repository through your own credentials (an SSH key or
`gh auth setup-git` for HTTPS), run:

```sh
claude plugin marketplace add helico-tech/cicada.rs
claude plugin install cicada@cicada
```

Start a new Claude Code session and look for `/cicada:cicada` and `/cicada:actor-design`. The shell command installs
at user scope; [Claude Code's plugin guide](https://code.claude.com/docs/en/discover-plugins) covers other scopes and
Git access failures.
The plugin follows `main`: its version matches the workspace, but its files can include documentation committed
after that release tag. It has no `SOURCE.json`, so the helper reports `revision_match: unknown`.

**Release archive.** For other agents, or the exact skill files of a release tag, use the archive attached to a
published GitHub Release. Archives start at 0.17.2; from 0.19.3 they contain both `cicada/` and `actor-design/`.
For a **first install** into a project without either skill folder:

```sh
staging=$(mktemp -d)
gh release download v0.24.0 --repo helico-tech/cicada.rs --pattern 'cicada-skill-*.tar.gz' --dir "$staging"
tar -xzf "$staging/cicada-skill-v0.24.0.tar.gz" -C "$staging"
# Inspect both skills, the helper and the design template before copying them.
mkdir -p .claude/skills
cp -R "$staging/cicada" "$staging/actor-design" .claude/skills/
```

The directory shown is for Claude Code; use your agent's project skill directory. Keep both folders side by side
at the same version: `actor-design` runs the resolver in its sibling `cicada`. Installing `cicada` alone remains valid;
archives before 0.19.3 contain only that folder. Each archived skill's `SOURCE.json` identifies the same release commit.

From the consumer project, the installed helper can report the matching sources
(use the actual installed skill folder, quoted for the shell):

```sh
python3 ".claude/skills/cicada/scripts/cicada_source.py"
```

Use the actual target/features when they differ from the host defaults. The skill's [reading map](../agents.md)
explains source selection. Equal versions can have different commits or local changes; your resolved source wins.
Without Python/toolchain, inspect Cargo.lock and verifiable local source manually rather than guessing cache paths.

### Update the installed skill

**Plugin:** update both skills through Claude Code's `/plugin`. Updates fetch a new copy when the plugin version changes
with a Cicada release; documentation-only commits keep that version. Edits in the plugin cache do not survive updates.

**Archive:** download and unpack the new archive into a fresh staging directory as above. Nothing updates an archive
installation automatically. Compare before replacing it:

```sh
diff -r "$staging/cicada" .claude/skills/cicada
diff -r "$staging/actor-design" .claude/skills/actor-design
```

When updating from an archive before 0.19.3, the missing installed `actor-design` folder is expected. After reviewing
the differences and preserving local changes you need, replace both folders whole so stale files do not remain:

```sh
rm -rf .claude/skills/cicada .claude/skills/actor-design
cp -R "$staging/cicada" "$staging/actor-design" .claude/skills/
```

To switch from an archive to the plugin, compare both installed folders with the plugin's skill folders and save
local changes you need elsewhere. Then remove the archive copies so only one installation is active. Keeping the
archive is also fine; switching installations and enabling automatic updates are your choices.

Updating the skills does not change your Cicada dependency, saved designs or review reports. A later review of the
same system and scope resumes its report, keeping finding ids and your notes; findings that depend on changed code
or Cicada source become stale until checked again. The project's resolved source still wins with either installation.
Projects on releases before 0.17.2 can use newer skills against their own older source; those releases have no skill
archive.
