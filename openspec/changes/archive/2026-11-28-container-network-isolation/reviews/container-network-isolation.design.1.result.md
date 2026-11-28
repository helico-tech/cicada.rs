Verdict: not yet

## Blocking

1. **The version refusal can be read as create-only, which fails open.** Where: design.md Decisions 4 and 6, and the container-docker spec "Docker refuses isolation below API 1.48". Decision 4 says an old daemon fails "before creating anything". But Decision 6 puts the refusal in the pure function that builds the create request. In `ensure_network` (docker/mod.rs), that request is built only after the existing-network lookup. A Docker 27 daemon keeps unknown bridge options and shows them in inspect: a network made by hand with the option (as Factory did), or one left over after a downgrade from 28. Decision 5 would then count that network as isolated and return `Ok`, but its bridge has a host address. Fix: refuse right after the `InvalidSpec` check and before the lookup, and add a scenario: "an old daemon, and an internal network of that name exists with both options → `Unsupported`".

## Notes

1. Design.md Decision 7: `Networking` has no host variant. Say that the listener container is started with the docker CLI (as `docker run` already is at container_docker.rs:742), not through a new `Networking` variant.
2. Decision 7 and the spec scenario: say which address the isolated container probes. Isolated mode puts no address on the bridge, and IPAM may report no gateway. Use the first address of the subnet, for example.
3. A host's own `ContainerRuntime` written before this change ignores `isolated` and returns `Ok` for an open network. Name this in the CHANGELOG line and in the trait doc. Also update the `ensure_network` doc ("Only the `internal` flag…", runtime.rs:177) and the `Unsupported` doc (error.rs).
4. Docs handoff: name the upgrade step. Existing internal networks get `Conflict` once a host asks for `isolated`, so they must be removed first.

## Questions for the owner

None.

## Not checked

I did not run any build, test or E2E. I did not check against a daemon that Docker 28+ accepts `gateway_mode_ipv6=isolated` on a network without IPv6, or that inspect shows both options. I did not check what bollard's highest client API version is.
