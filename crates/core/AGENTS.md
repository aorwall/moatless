# crates/core

## Migrations

The backend runs `migrations/` at startup (`RUN_MIGRATIONS`), on Kubernetes
in the `migrate` init container first, while the replica on the previous
release still serves: a rolling update replaces pods one at a time. Every
migration must therefore keep that previous release working.

- Add before you read: a new column ships nullable or with a default.
- Contract one release late. Remove every reader and writer of a table or
  column in one PR; drop or rename it in a later PR, after that one is
  deployed. Its migration carries a line `-- contract-after: #<that PR>`, which
  `migration-contract` in `tooling/arch-rules` requires on any `DROP TABLE`,
  `DROP COLUMN` or `ALTER TABLE … RENAME`.
- Never edit an applied migration: production records its checksum.
- Write a backfill as one set-based statement: aggregate a large table once
  and join it, never a correlated subquery per row. Each migration is one
  transaction, so a slow one holds its locks and the rollout until it ends.
  `EXPLAIN` it against nonprod's row counts before you merge.
