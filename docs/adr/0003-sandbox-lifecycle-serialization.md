# Sandbox lifecycle serialization and ordering

The `sandbox::lifecycle` module is a durable, level-based controller: persisted
desired state, a correlation marker, deterministic events, and a reconciler.
Operations are **at-least-once with idempotent effects** — not a durable queue
with exactly-once execution.

**Per-Task serialization is a session advisory lock on a dedicated pool.** Every
operation takes `pg_try_advisory_lock` keyed on the Task, on a lock-only
connection pool separate from the main pool, with bounded retry and the
connection released between attempts; exhausted retries surface
`LifecycleError::OperationInProgress { kind }` carrying the conflicting
operation's kind. The holder keeps its lock connection for the operation's whole
duration while lifecycle DB writes go through the main pool. A lock connection
whose unlock fails, or that drops mid-operation, is **closed rather than returned
to its pool**, so the session lock dies with the connection. Every operation
takes the lock — there is no lock-free fast path.

**Intent is written before the runtime effect, in one atomic statement.** Desired
state, stop reason, archive retention, and the correlation marker
(`kind` + `operation_generation` + `started_at`) change in a single statement, so
readers never observe partial intent; a CTE captures the pre-image so the write
reports whether the desired-state / reason / retention actually changed (the
intent-change axis) independently of the marker, which always advances. If the
runtime effect then fails, intent remains the requested target and the reconciler
(level operations) or the caller (edges) retries.

**The correlation marker outlives the runtime call.** Kubernetes returns
`Starting` before readiness and Docker events arrive asynchronously, so clearing
the marker on return would misclassify late events. The `operation_generation` is
a durable per-Task counter (`sandbox_operation_seq`): a new operation allocates
the next generation, recovery of an incomplete operation reuses its marker
generation. The generation is stamped onto the resources an operation creates and
carried on every watcher event, so a superseded observation is distinguished by
identity, not the clock. The reconciler is the marker's acknowledgement owner;
marker age is a recovery signal, never proof of completion.

**Completion facts are deterministic and ride the transactional outbox.**
Operation events use the id `sandbox:{task_id}:{operation_generation}:{fact}` over
the idempotent-persistence path (`ON CONFLICT (event_id) DO NOTHING`), so
at-least-once retries of one generation persist once. A completion fact
(`Stopped` / `Removed`) is persisted and the marker cleared in **one transaction
before publication**; a failed completion commit is a typed error, so a
deterministic event id is never live without its persisted fact.

## Considered options

**Transaction-scoped advisory lock (`pg_advisory_xact_lock`).** Rejected: it holds
a database transaction open across the runtime effect, which can wait tens of
seconds on Kubernetes provisioning — pinning a transaction and a connection for
the whole transition.

**Blocking `pg_advisory_lock` on the shared pool.** Rejected: a blocking acquire
on the shared (default five-connection) pool can starve every other query behind
contended Tasks. The try-lock with bounded retry on a dedicated pool bounds the
blast radius to the lock pool and surfaces contention as a typed error the caller
can react to.

**A durable operation queue with exactly-once execution.** Rejected as
unnecessary: desired state is a level, and idempotent effects converge under
at-least-once retries driven by the reconciler. A queue adds exactly-once
machinery the level-based controller does not need.

**One `UPDATE` per patched field (today's DAO shape).** Rejected: separate
statements for desired state and stop reason let a reader observe a half-applied
stop (new desired state, stale reason). The single-statement write with a
pre-image CTE keeps intent atomic and still reports the intent-change axis.

**Heuristic event de-duplication.** Rejected: deterministic generation-scoped ids
over the existing outbox dedup exactly, including a reconciler that persists a
missing completion fact and clears its marker in one transaction after a crash —
without a fuzzy "have I seen this?" check.
