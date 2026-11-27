Verdict: ready for tasks

Blocking: none.

1. Lock refusal: design.md item 3 now refuses a config path named, or resolving to, tray.toml or tray.toml.lock. Holds; a config's own lock (<target>.lock) cannot then collide with the tray's lock.
2. Lone all/none type: holds against pick_types. "all" returns every hosted type, so a lone type named all or none runs exactly that type (the worker-with-none error is not reached).
3. Spec scenario now covers all and none.

Notes: no tray-settings task text mentions the lock-file refusal; tasks should add a test for it.

Not checked: the code, only the design and spec text.
