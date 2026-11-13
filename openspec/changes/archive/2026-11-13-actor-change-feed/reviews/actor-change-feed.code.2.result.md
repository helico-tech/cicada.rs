Verdict: pass

Blocking findings: none. Both round-1 blockers are fixed.

- SQLite: the new shutdown test retains the Store on a surviving runtime, stops the create at each commit barrier, and destroys the other runtime. The bounded next write verifies release of the writer guard and rollback-on-next-use; the queued case must disappear and the in-flight case must survive. The fresh subscription asserts initial Resync, reads the affected actor, and receives subsequent hints.
- PostgreSQL: the first attempt collects p-001 before rollback; the intervening plain-SQL update detaches it, so the successful attempt cannot collect it. The test checks its survival and exactly the other three hints. A collector retained across attempts would now fail that assertion. The hook runs after rollback, releases its mutex before awaiting, and remains test-only.

Notes:
- openspec/changes/actor-change-feed/tasks.md:41 still says runtime shutdown during settlement is not tested. Update that historical coverage note to reflect the new test.

Not checked: tests or other finishing checks were not run; trusted the builder's reported results. Review was limited to 4af1aac..fad1f59 and supporting implementation/spec context. No mutation test was run. The previously noted listener probe-task issue is outside this fix diff and was not re-reviewed.
