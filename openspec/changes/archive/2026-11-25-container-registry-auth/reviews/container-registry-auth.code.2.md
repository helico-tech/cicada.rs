---
id: container-registry-auth.code.2
kind: code
change: container-registry-auth
round: 2
base: e39fd91222da62d8b98a3fcb4b101672a2812542
head: fb95ff4e7067d4b2f75fd5be86afb1f56855790d
previous: 944c2d7f4b59dd412047cc02d131eb337a68b7bf
requested: 2026-11-25T09:59:54+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done container-registry-auth.code.2 <file>`.

## Note from the requester

Version: 0.21.1

Round 2 fixes (head fb95ff4e): (1) the login-after-connect E2E step makes the runtime with docker_config before config.json
exists, calls info(), checks the file is still missing, then logs in, pushes and pulls with that same runtime; (2) new E2E
step a_public_image_is_pulled_past_a_helper_that_is_not_installed (busybox removed first, credsStore
"cicada-no-such-helper", create succeeds, own temp folder); (3) unit test a_configured_helper_gives_the_credentials_instead_of_auths;
Config::credentials takes an optional helper folder for tests, the backend passes None (PATH); (4) handoff: only
$HOME/.docker without DOCKER_CONFIG or docker_config. Checks: cicada-containers all features passed; container E2E steps 30
passed; fmt, clippy -D warnings clean. Gate not run: it waits for pool-warning-test-race (unrelated flaky test), then this
branch is rebased and gated. Do not block on the missing gate evidence.
