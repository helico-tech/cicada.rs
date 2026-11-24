Verdict: fix needed

## Blocking

1. `crates/cicada-containers/src/runtime.rs` (rustdoc of `make_dirs`), `docs-handoff.md` §2–3, `CHANGELOG.md`, spec `container-docker`: "more than 8 links in a row is `Conflict`" is untrue on Docker. Docker's archive stat returns the link target already fully resolved, as an absolute path. So a chain of any length is one hop, and a link loop comes back as HTTP 500, which `kind_of` maps to `Other`. How to see it: in a created `alpine:3` container with `/tmp/l1 -> l2 -> l3 -> /tmp` and `loopa <-> loopb`, `HEAD .../archive?path=/tmp/l1` gave `linkTarget: "/tmp"`, and `?path=/tmp/loopa` gave 500. Only the fake stat in `dirs.rs` tests the hop limit. Fixed means one of two things. (a) The rustdoc, handoff and CHANGELOG say what Docker does: Docker resolves a chain itself, and a loop fails with `Other`. (b) The backend maps a looping stat to `Conflict`, and an E2E step pins it. I prefer (a): it is smaller, and the walk's limit stays as a guard.

## Checked and correct

The downward walk is right. Each statted path is the existing, link-free parts plus one part. A link's target goes in front of the remaining named parts, so `missing` holds only named parts. Nothing below a file is ever statted. Every check runs before the upload. The tar holds only missing folders, under resolved paths, and the path rule and mount grouping use those paths. Tasks are ticked, the scenarios have tests, the version is 0.20.9 and no protected docs are touched.

## Notes

1. The links counter counts all links in the walk, not "in a row". The wording should say so.
2. The relative-link tests model raw targets that Docker never sends. Harmless.

## Not checked

I did not run the suites, the E2E or the gate.
