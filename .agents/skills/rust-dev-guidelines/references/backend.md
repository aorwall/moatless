# Backend Rust

Apply this reference to Rust changes under `backend/`.

## Module and State Boundaries

- Organize modules around cohesive responsibilities and keep dependencies
  pointing toward domain behavior. Do not impose a fixed file template.
- HTTP and webhook handlers obtain persistence and domain behavior through DAOs
  or services injected into `AppState`.
- Construct concrete `Pg*` DAO implementations in composition roots or inside
  DAO/service implementations, not in handlers. A pool-wrapper DAO may be built
  at the root or locally as convenient (a provider adapter that carries its own
  ports builds its instance where it is constructed) precisely because it must
  stay a stateless wrapper over the pool — two instances never diverge. A DAO
  that acquires in-memory state (a dedup cache, a counter, a notify channel) has
  to be built once and shared instead, not rebuilt per consumer.
- Direct pool access in handlers is limited to documented authentication,
  infrastructure, or transaction boundaries.
- Add a trait only when the boundary has more than one real consumer or a test
  uses a mock/fake implementation.

## Persistence Naming

- A persistence trait is `*Dao`; its Postgres implementation is `Pg*Dao`, and
  both live in the owning module's `dao.rs` — `TaskDao`/`PgTaskDao` in
  `task/dao.rs`. The acronym is camel-cased like `ApiError` and `DbPool`, so it
  is `Dao`, never `DAO`.
- `Repository` names the git-remote domain entity only. `Store` is not a
  type-name suffix.
- `AppState` field names stay domain-plural (`state.tasks`, `state.users`); the
  `Dao` suffix lives on the type they hold, not on the field.

## SQL and Pagination

- Bind data values. Dynamic SQL may vary only through validated structural
  fragments before `AssertSqlSafe`.
- Match fetch cardinality to the query: optional for zero-or-one, one only when
  guaranteed, and bounded list reads for collections.
- Choose explicit projections when they protect a contract or avoid ambiguous
  joins; `SELECT *` is not categorically forbidden.
- A paginated query must count the same distinct filtered entity set returned
  by the page and report the total even when the requested page is empty.
- Patch semantics must distinguish omitted fields from explicit clearing where
  the API supports both.

## HTTP and Errors

- Axum's body-consuming extractor is last. Non-body extractors have no
  repository-mandated order.
- Handlers coordinate transport concerns; move reusable domain behavior into an
  owned service or DAO operation.
- Keep request/response DTOs and OpenAPI annotations aligned with the public
  response contract.
- Internal failures retain their source. `IntoResponse` owns stable status
  mapping and redaction, and logs the causal failure once.
