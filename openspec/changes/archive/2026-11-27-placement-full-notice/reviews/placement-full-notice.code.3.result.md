Verdict: pass

Both round 2 findings are resolved. The full-pool branch leaves node entries alone, and `end_reports` can clear an expired conflict mark before the offer is handled. Known nodes retain the existing renewal, instance-conflict, and resource-contest paths. Line 71 quotes the existing guide in the “Where” field.

Reviewed commit `24c29648` against the actor source. Tests and the gate were not run.
