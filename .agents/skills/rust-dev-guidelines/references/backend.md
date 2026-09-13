# Backend Rust

Apply this reference to backend Rust across `crates/`. Read [where a type belongs](file-roles.md#choose-a-home-for-a-deduplicated-shared-type) and the owning crate's guidance before selecting a module; `crates/server` composes domain crates.

## Module and State Boundaries

- Follow the [file responsibilities](file-roles.md) loaded from `SKILL.md`.
  Keep dependencies pointing toward domain behavior.
- HTTP and webhook handlers obtain persistence and domain behavior through DAOs
  or services injected into `AppState`.
- Construct concrete `Pg*` DAO implementations in composition roots or inside
  DAO/service implementations, not in handlers. A pool-wrapper DAO may be built
  at the root or locally as convenient (a provider adapter that carries its own
  ports builds its instance where it is constructed) precisely because it must
  stay a stateless wrapper over the pool — two instances never diverge. A DAO
  that acquires in-memory state (a dedup cache, a counter, a notify channel) has
  to be built once and shared instead, not rebuilt per consumer.
- Add a trait only when the boundary has more than one production implementation or a test
  uses a mock/fake implementation.

## Persistence Naming

- A persistence trait is `*Dao`; its Postgres implementation is `Pg*Dao`, and
  both belong to the owning module's `dao` module — `TaskDao`/`PgTaskDao` in
  `task/dao.rs` or its private implementation children. A directory root
  declares and re-exports; it contains no implementation bodies.
  Check architecture-rule path assumptions before changing the file layout.
  The acronym is camel-cased like `ApiError` and `DbPool`, so it
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

## Integration Tests

Put domain and provider tests in their owning crate. Inspect its Cargo targets and
`db-tests` feature before selecting a command; plain unit tests omit gated cases.
The backend composition suite has three directory targets in `crates/server/tests`:
`integration_data`, `integration_adapters`, and `integration_surface`, each with
its own `main.rs`. Add modules to the appropriate target and reuse `crate::common`.
Do not move tests between targets without measuring compile cost.

Follow the exact local commands in root AGENTS.md. CI discovers db-tests packages
from Cargo metadata; it also compiles the gated tier. Read
[build costs](build-costs.md) before changing the verification matrix.

## HTTP and Errors

- Axum's body-consuming extractor is last. Non-body extractors have no
  repository-mandated order.
- Keep request/response DTOs and OpenAPI annotations aligned with the public
  response contract.
- Internal failures retain their source. `IntoResponse` owns stable status
  mapping and redaction, and logs the causal failure once.
