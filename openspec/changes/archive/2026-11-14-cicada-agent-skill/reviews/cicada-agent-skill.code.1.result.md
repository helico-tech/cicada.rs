Verdict: fix needed

Blocking findings:
1. skills/cicada/scripts/cicada_source.py:262–268 (and SKILL.md:38–39): unavailable Git inspection becomes a false exact match. In an outside-repository fixture with Git absent from PATH and Cargo's revision equal to SOURCE.json, discover returned commit=null, dirty=null, modified=false, revision_match="same". The skill declares that pair the same code despite unverified local contents. Preserve unknown modification/provenance state, require successful HEAD and cleanliness verification before claiming an exact checkout match, and test missing/failed Git probes.
2. docs/guide/core.md:280 says a timeout runs the handler again independently of sender retry. Architecture's turn-outcome table and existing a_turn_timeout_ends_failed_timeout_by_default establish the opposite: without an applicable retry policy it fails; blocking overtime is never retried. Qualify timeout repetition by retry policy and distinguish it from abandoned claims.
3. README.md:195: the install command fails in a fresh consumer project because cp cannot create missing .claude/skills parents. Create the parent directory before copying; verify the documented installation in an empty project. This is the Install and update scenario's normal first-install path.

Notes: Packaging uses tag objects, deterministic metadata and existing release gates correctly on inspection. Three of nine behavioral evaluations ran; the remaining six are explicitly disclosed, consistent with the agreed representative subset.

Not checked: suites, compilation, PostgreSQL, E2E, live GitHub Actions, or fresh model evaluations. Builder check results are reported evidence, not independently reproduced. Reviewed the specified commits, artifacts, tests and evaluation record; ran only the focused provenance experiment outside the repository.
