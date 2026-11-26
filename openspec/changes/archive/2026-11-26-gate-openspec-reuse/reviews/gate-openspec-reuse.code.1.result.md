Verdict: pass

## Blocking findings

None.

## Safety of the allowlist

I found no way for a commit that changes code to reuse a full pass:
- `classify` is unchanged. It accepts only A/M/D of mode 100644 and runs `--no-renames`, so a move out of `src/` still shows as a refused delete. Symlinks, submodules, type changes and executables are refused everywhere. If `openspec` itself became a symlink, the new entry is `openspec` (not `openspec/...`), so it is refused.
- No build or test input lives under `openspec/`, and nothing points into it. Cargo members are listed by name, and no path dependency, `include!`, `include_str!` or `#[path]` targets it. The tree has no symlinks at all. Cargo and rustfmt config, `rust-toolchain` and `.dockerignore` are found from the root, not from `openspec/`. Python discovery covers `scripts/` only. `status.py` and `review.py` read `openspec/`, but the python-tests in the docs kind cover them. Docker `COPY . .` takes `openspec/`, but the cargo build never reads it.
- The policy file is outside `openspec/`, and changing it forces a full run.
- What is left is the risk ADR 0070 already names: a later full-passed commit that starts reading `openspec/` would open a hole. Only review can catch that.

## Notes

1. Both markers of this change (`design.1.taken`, `code.1.taken`) are 0755. Main's old `review.py` wrote them. So this change's archive commit, and that of any change whose reviews were taken before landing, still carries executable markers and needs a full run. Fix: `chmod 644` at landing, or a line about it in the handoff.
2. `parse_policy` does not require a trailing `/` on prefixes. With `"openspec"` (no slash), any path starting `openspecX` would also match. The risk is low because the policy is reviewed.
3. The handoff's proposed sentence leaves out the listed root files (README, CHANGELOG, AGENTS). That is minor wording.

## Not checked

I did not run the script tests, the gate or `openspec validate`; I rely on the builder's report. I did not check that the new marker test fails without the fix.
