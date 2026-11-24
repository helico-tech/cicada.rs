---
id: container-runtime.code.1
kind: code
change: container-runtime
round: 1
base: b727f081811aaf5a61f819815105ed5126da0bc7
head: c49f68b0f99444fc7c4d19b3a0302368e6a43b75
requested: 2026-11-24T09:58:25+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-runtime.code.1 <file>`.

## Note from the requester

Version: 0.20.7

Builder report (head c49f68b, 5 commits on b727f08): tasks 1-5 done. cicada-containers tests 21 passed (+1 doctest) with and without testing; clippy clean; cargo test --workspace 3142 passed (PostgreSQL tests skipped: Docker was down); feature-builds.sh and dependency-trees.sh pass; script tests 102; skill 18; docs 11; fmt, clippy workspace, openspec strict pass. The full gate on c49f68b is run by the change loop after this review starts; its result will be added before landing.
Choices beyond the design: Call::Create holds Box<ContainerSpec> (clippy size); command is Option<Vec<String>> like entrypoint; the fake checks copy_in paths (relative, read-only root) as decision 9 says a backend refuses; added names MountKind, Skipped, Resource.
Docs: handed off, nothing blocks.
