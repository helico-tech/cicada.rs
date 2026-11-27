Verdict: not yet

Blocking findings in design.md

1. D3 places tray.toml beside the supplied path. If config.toml links to that tray.toml, Apply overwrites credentials. Reject resolved collisions. Keep zero-type refusal and make it a spec exception. Test literal TOML keys.
2. D2/5 need one lock keyed by the resolved config target. Merge dirty fields under it. Serialize tray read–modify–write too. The partial-save message is defined. Require retry without reconnecting on failure.
3. D4 correctly overrides worker environment settings and preserves server/token precedence. In settings.rs singleton all expands and none fails. Encode literal singletons unambiguously.
4. D1 defines enumeration/current/lookup and sanitized loading. Specify path access, first-set selection and CLI header replacement. Redact ContextFile and entry Debug, including header values and URL passwords as Target does.
5. D5 defines the name rule, shared URL check and 1,000 ceiling. Reject duplicate Add names against fresh data under lock.

D6 resolves reconnect through separate apply, pending-pick replacement and Quit priority.

Still fits five tasks: shared config, tray persistence/form, worker arguments, reconnect, AppKit.

OpenSpec validation passed. Runtime tests and macOS checks not run.
