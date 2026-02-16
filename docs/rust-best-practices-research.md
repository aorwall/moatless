# Rust Best Practices Research: Backend & Sandbox Code Review

> Deep analysis of the moatless-vibe Rust codebase against modern Rust idioms and safety best practices.
> Covers: `backend/`, `sandbox/`, `crates/claude-agent-sdk/`, `crates/events/`, `crates/sandbox-proto/`

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Architecture Assessment](#architecture-assessment)
3. [Error Handling](#error-handling)
4. [Type Safety & API Design](#type-safety--api-design)
5. [Async Patterns & Concurrency Safety](#async-patterns--concurrency-safety)
6. [Database Access Patterns](#database-access-patterns)
7. [Resource Management](#resource-management)
8. [Code Organization & Modularity](#code-organization--modularity)
9. [Testing Patterns](#testing-patterns)
10. [Security Considerations](#security-considerations)
11. [Prioritized Recommendations](#prioritized-recommendations)

---

## Executive Summary

The Rust backend is **well-structured** with clear module boundaries, good use of trait-based abstractions for testability, and correct async patterns overall. The codebase demonstrates solid Rust foundations: `thiserror` for error types, `async_trait` for repository traits, `CancellationToken` for graceful shutdown, and proper use of `Arc` for shared state.

However, there are several areas where the code could be more idiomatic, safer, and more maintainable. The findings are organized by severity:

### Critical (should fix)
- Dynamic SQL construction in `update_task` bypasses SQLx compile-time safety
- `Box<dyn std::error::Error>` used broadly where typed errors would be safer
- Several `unwrap()`/`expect()` calls in non-test production paths (startup, metrics, auth clients)
- Spawned tasks without `JoinHandle` tracking (fire-and-forget without crash recovery)

### Important (should improve)
- N+1 query patterns in `resolve_user_display_names` and `resolve_repo_names`
- Repetitive error types across modules that could share a common trait or macro
- `String` used as identifiers throughout — newtypes would prevent ID mixing
- Excessive `.clone()` calls (392 occurrences across 49 backend files)
- Manual `sqlx::Type`/`Encode`/`Decode` impls for enums that could use `sqlx::Type(rename_all)`
- `async_trait` used in 23 files — could be replaced with native async traits (Rust 1.75+)

### Nice to Have (improve incrementally)
- Configuration could use validation at parse time
- More granular visibility (`pub(crate)` vs `pub`)
- Handler code duplication (create_task / create_vibe_task)
- Builder pattern for large struct construction (TaskMessage has 30+ fields)

---

## Architecture Assessment

### What's Working Well

**1. Trait-based Repository Abstraction**
The `AppState` uses `Arc<dyn Repository>` for all data access, enabling mock-based testing:
```rust
// state.rs - Excellent pattern
pub struct AppState {
    pub tasks: Arc<dyn TaskRepository>,
    pub messages: Arc<dyn MessageRepository>,
    // ...
}
```
This is textbook Rust — trait objects for DI, `Arc` for shared ownership across async tasks. The `#[cfg_attr(test, mockall::automock)]` annotation is well-applied.

**2. Consistent Module Structure**
Each domain module follows the same pattern: `mod.rs` → `models.rs` → `dao.rs` → `schemas.rs` → `handlers.rs` → `routes.rs` → `error.rs`. This consistency reduces cognitive load.

**3. Clean Startup Decomposition**
`startup.rs` breaks initialization into focused functions (`init_database`, `init_event_publisher`, `init_sse`, etc.) rather than a monolithic main function.

**4. CancellationToken for Graceful Shutdown**
Proper use of `tokio_util::CancellationToken` propagated to all background tasks.

### Areas for Improvement

**1. The `AppState` God Object**
`AppState` holds 14 fields and growing. Consider splitting into domain-specific state types:

```rust
// Current: everything in one struct
pub struct AppState {
    pub db: DbPool,
    pub config: Arc<Config>,
    pub sandbox: Arc<dyn SandboxProvider>,
    pub events: Option<Arc<dyn EventPublisher>>,
    pub s3: Option<aws_sdk_s3::Client>,
    pub channel_providers: Option<Arc<ChannelProviders>>,
    pub tasks: Arc<dyn TaskRepository>,
    pub messages: Arc<dyn MessageRepository>,
    // ... 6 more repositories
}

// Better: compose domain-specific state
pub struct TaskState {
    pub tasks: Arc<dyn TaskRepository>,
    pub tags: Arc<dyn TagRepository>,
    pub repositories: Arc<dyn RepoRepository>,
}

pub struct AppState {
    pub config: Arc<Config>,
    pub task_state: TaskState,
    pub message_state: MessageState,
    pub sandbox: Arc<dyn SandboxProvider>,
    // ...
}
```

**2. Missing Graceful Shutdown for gRPC and HTTP Servers**
The gRPC servers in `startup.rs:175` and `sandbox/main.rs:72` are spawned but their `JoinHandle`s are dropped. If these tasks panic, the error is silently lost:

```rust
// startup.rs:175 — JoinHandle dropped
tokio::spawn(async move {
    // gRPC server runs here
    if let Err(e) = tonic::transport::Server::builder()...serve(addr).await {
        tracing::error!(error = %e, "gRPC server failed");
    }
});
```

**Recommendation**: Store `JoinHandle`s and integrate with the `CancellationToken` for coordinated shutdown:
```rust
let grpc_handle = tokio::spawn(async move { /* ... */ });
// In shutdown handler:
cancel.cancel();
grpc_handle.await??;
```

---

## Error Handling

### Strengths
- Consistent use of `thiserror` for all module error enums
- `IntoResponse` implemented on all error types for Axum integration
- Database errors are properly wrapped and don't leak internal details to clients
- `?` operator used correctly throughout the codebase

### Issue 1: `Box<dyn std::error::Error>` as Return Type

Found in 10 locations across the codebase:
- `backend/src/main.rs:26` — `main()` returns `Box<dyn std::error::Error>`
- `backend/src/startup.rs:115` — `init_sse()`
- `backend/src/sandbox/grpc_client.rs:31,39` — `for_service()` and `connect()`
- `backend/src/sandbox/idle_reaper.rs:96` — `reap_idle_sandboxes()`
- `backend/src/sse/consumer.rs:72` — `consume_cycle()`
- `backend/src/event_handler/consumer.rs:74` — `consume_cycle()`
- `sandbox/src/main.rs:29` — `main()`
- `sandbox/src/backend_client.rs:57` — `connect()`

**Problem**: `Box<dyn std::error::Error>` erases type information, making it impossible to handle specific error cases. It also can't be sent across thread boundaries unless `Send + Sync` is added.

**Recommendation**: For `main()` functions, this is acceptable. For library functions, define typed errors:
```rust
// Instead of:
pub fn connect(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> { ... }

// Use:
#[derive(Debug, thiserror::Error)]
pub enum GrpcClientError {
    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] tonic::transport::Error),
    #[error("Connection failed: {0}")]
    Connection(String),
}

pub fn connect(url: &str) -> Result<Self, GrpcClientError> { ... }
```

### Issue 2: `unwrap()` and `expect()` in Production Code

**Production `expect()` calls** (will panic if they fail):

| File | Line | Risk |
|------|------|------|
| `backend/src/main.rs` | 37 | `expect("Failed to install rustls crypto provider")` — acceptable startup check |
| `backend/src/metrics.rs` | 17 | `expect("Failed to install Prometheus recorder")` — acceptable startup check |
| `backend/src/startup.rs` | 51-53 | `unwrap()` on TcpListener bind and axum::serve — **panics silently in spawned task** |
| `backend/src/sandbox/k8s/leader_election.rs` | 143 | `expect("valid lease JSON")` — **can panic on malformed K8s data** |
| `backend/src/auth/oauth/auth0.rs` | 33 | `expect("failed to build reqwest client")` — acceptable (build-time config) |
| `backend/src/auth/oauth/github.rs` | 28 | `expect("failed to build reqwest client")` — acceptable (build-time config) |
| `sandbox/src/config.rs` | 147 | `expect("BACKEND_GRPC_URL is required")` — acceptable (required env var) |
| `sandbox/src/main.rs` | 51 | `expect("Failed to create backend gRPC client")` — acceptable startup check |

**Key concern**: `startup.rs:51-53` uses `unwrap()` inside a `tokio::spawn`:
```rust
tokio::spawn(async move {
    let listener = TcpListener::bind(addr).await.unwrap(); // panics in spawned task!
    axum::serve(listener, router).await.unwrap();
});
```
A panic in a spawned task aborts only that task, so if the metrics server fails to bind, the main application continues running without metrics — silently.

**Recommendation**: Use `expect()` with descriptive messages, or propagate errors:
```rust
tokio::spawn(async move {
    let listener = TcpListener::bind(addr).await
        .expect("Failed to bind metrics server — is the port in use?");
    if let Err(e) = axum::serve(listener, router).await {
        tracing::error!(error = %e, "Metrics server failed");
    }
});
```

### Issue 3: Silently Dropped Results (`let _ = ...`)

Found 8 instances in production code:
- `backend/src/config.rs:237` — `let _ = dotenvy::dotenv()` — acceptable
- `backend/src/startup.rs:205` — `let _ = tokio::signal::ctrl_c().await` — acceptable
- `backend/src/channel/slack/provider.rs:297` — reaction removal — acceptable (best-effort)
- `backend/src/repository/handlers.rs:315` — **`let _ = state.repositories.update(...)` — silently drops DB update error**
- `backend/src/sse/consumer.rs:211` — broadcast send failure — acceptable (no receivers)
- `backend/src/sandbox/k8s/provider.rs:432` — **`let _ = deployment::scale_deployment(...)` — silently ignores K8s failure**

**Recommendation**: For the two problematic cases, at minimum log a warning:
```rust
// Instead of:
let _ = state.repositories.update(&repository_id, &patch).await;

// Use:
if let Err(e) = state.repositories.update(&repository_id, &patch).await {
    tracing::warn!(error = %e, "Failed to update repository after operation");
}
```

### Issue 4: Repetitive Error Types

`TaskError`, `MessageError`, `SandboxError`, `ChannelError`, `FigmaError`, `FileStorageError` all follow the same pattern:
```rust
#[derive(Debug, thiserror::Error)]
pub enum XxxError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Invalid request: {0}")]
    BadRequest(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}
```

**Recommendation**: Use a macro to reduce boilerplate:
```rust
macro_rules! define_api_error {
    ($name:ident $(, $extra_variant:ident($extra_ty:ty) => $extra_status:expr)*) => {
        #[derive(Debug, thiserror::Error)]
        pub enum $name {
            #[error("Not found: {0}")]
            NotFound(String),
            #[error("Invalid request: {0}")]
            BadRequest(String),
            #[error("Database error: {0}")]
            Db(#[from] sqlx::Error),
            #[error("Internal error: {0}")]
            Internal(String),
            $(
                #[error("{0}")]
                $extra_variant($extra_ty),
            )*
        }
    };
}
```

Or better, use a shared base error type with `From` implementations.

---

## Type Safety & API Design

### Issue 1: String-typed Identifiers (Most Impactful)

Task IDs, user IDs, repository IDs, session IDs, and message UUIDs are all represented as `String`. This means the compiler cannot prevent accidentally passing a `user_id` where a `task_id` is expected:

```rust
// Current: compiler allows mixing IDs
pub async fn find_by_id(&self, task_id: &str, user_id: &str) -> ...;

// Easy to make this mistake with no compiler error:
state.tasks.find_by_id(&user_id, &task_id) // swapped!
```

**Recommendation**: Use the newtype pattern:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

// Now the compiler catches mistakes:
pub async fn find_by_id(&self, task_id: &TaskId, user_id: &UserId) -> ...;
```

This is a large refactor but yields significant safety benefits. Could be done incrementally, starting with `TaskId` which is the most commonly used.

### Issue 2: Timestamps as Strings

`Task.created_at` and `Task.last_message_at` are `String` fields, while `Task.created_at_db` is `Option<DateTime<Utc>>`. This inconsistency forces repeated parsing:

```rust
pub struct Task {
    pub created_at: String,          // RFC3339 string
    pub last_message_at: String,     // RFC3339 string
    pub created_at_db: Option<DateTime<Utc>>, // proper type
}
```

**Recommendation**: Use `DateTime<Utc>` everywhere, serialize to RFC3339 in the schema layer:
```rust
pub struct Task {
    pub created_at: DateTime<Utc>,
    pub last_message_at: DateTime<Utc>,
}

// In schemas.rs:
pub struct TaskResponse {
    #[serde(serialize_with = "serialize_rfc3339")]
    pub created_at: DateTime<Utc>,
}
```

### Issue 3: Manual SQLx Type Implementations

`TaskStatus`, `Visibility`, `MessageType`, `MessageRole`, `TurnStatus`, and `ContentBlockType` all manually implement `sqlx::Type`, `sqlx::Encode`, and `sqlx::Decode`. Each requires ~30 lines of boilerplate per enum.

**Recommendation**: Use `sqlx::Type` derive macro with `rename_all`:
```rust
// Instead of 30+ lines of manual impls:
#[derive(sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Closed,
    Error,
}
```
This compiles to the exact same behavior but eliminates ~180 lines of boilerplate across the model files.

### Issue 4: Large Struct Construction Without Builder

`TaskMessage` in `message/send.rs` has 30+ fields, leading to verbose construction with many `None` values:

```rust
let msg_row = TaskMessage {
    id: 0,
    task_id: task_id.to_string(),
    uuid: msg_uuid,
    message_type: MessageType::User,
    user_type: "human".to_string(),
    parent_uuid: None,
    is_sidechain: None,
    cwd: None,
    session_id: None,
    // ... 20+ more fields set to None
};
```

**Recommendation**: Add `Default` impl and a builder-like constructor:
```rust
impl TaskMessage {
    pub fn new_user_message(task_id: String, uuid: String, content: serde_json::Value) -> Self {
        Self {
            task_id,
            uuid,
            message_type: MessageType::User,
            user_type: "human".to_string(),
            message_role: MessageRole::User,
            message_content: content,
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ..Default::default()
        }
    }
}
```

### Issue 5: Handler Code Duplication

`create_task` and `create_vibe_task` share ~70% identical code (task creation, sandbox provisioning, initial message sending). This violates DRY:

```rust
// Both handlers do:
// 1. Validate repository exists
// 2. Generate task_id + short_id
// 3. Build Task struct
// 4. Insert task
// 5. Publish event
// 6. Provision sandbox
// 7. Send initial message
// 8. Enrich and return
```

**Recommendation**: Extract a shared `create_task_internal` helper that both handlers call with their specific configuration.

### Issue 6: `list_tasks` Has Too Many Parameters

```rust
async fn list_tasks(
    &self,
    user_id: &str,
    limit: i64,
    offset: i64,
    status: Option<&str>,
    repository_id: Option<&str>,
    created_by: Option<&str>,
    tag: Option<&str>,
) -> Result<(Vec<Task>, i64), sqlx::Error>;
```

7 parameters — suppressed with `#[allow(clippy::too_many_arguments)]`.

**Recommendation**: Use a filter struct:
```rust
pub struct TaskFilter<'a> {
    pub user_id: &'a str,
    pub limit: i64,
    pub offset: i64,
    pub status: Option<&'a str>,
    pub repository_id: Option<&'a str>,
    pub created_by: Option<&'a str>,
    pub tag: Option<&'a str>,
}

async fn list_tasks(&self, filter: &TaskFilter<'_>) -> Result<(Vec<Task>, i64), sqlx::Error>;
```

---

## Async Patterns & Concurrency Safety

### Strengths
- No `std::sync::Mutex` in async code (good — only `tokio::sync::RwLock` is used)
- `CancellationToken` properly propagated through the system
- `tokio::sync::Notify` used correctly for consumer wakeups
- Channel usage is clean (broadcast for SSE fan-out)

### Issue 1: Fire-and-Forget `tokio::spawn` Without Error Handling

9 instances of `tokio::spawn` in the backend, most without `JoinHandle` tracking:

```rust
// startup.rs:50 — metrics server
tokio::spawn(async move { /* panics lost */ });

// startup.rs:175 — gRPC server
tokio::spawn(async move { /* panics lost */ });

// sandbox/idle_reaper.rs:42
tokio::spawn(async move { /* panics lost */ });

// channel/handlers.rs:117
tokio::spawn(async move { /* panics lost */ });
```

**Problem**: If a spawned task panics, the `JoinHandle` is dropped and the panic is silently swallowed. The main task continues running in a degraded state.

**Recommendation**: For critical background services (gRPC, SSE consumer, idle reaper), use `tokio::JoinSet` or store handles:
```rust
let mut critical_tasks = tokio::task::JoinSet::new();
critical_tasks.spawn(async move { /* gRPC server */ });
critical_tasks.spawn(async move { /* SSE consumer */ });

// In shutdown or monitoring:
while let Some(result) = critical_tasks.join_next().await {
    if let Err(e) = result {
        tracing::error!(error = ?e, "Critical background task failed");
    }
}
```

### Issue 2: Agent Execution Spawn Chain in Sandbox

In `sandbox/src/agent/claude_code.rs`, `spawn_agent_execution` spawns a task, and within that task, spawns another task for the notification delay:

```rust
pub fn spawn_agent_execution(state: SandboxState, request: AgentRunRequest) {
    tokio::spawn(async move {
        // ... agent runs ...
        // Then spawns another task:
        let notify = state.consumer_notify.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            notify.notify_one();
        });
    });
}
```

**Problem**: The inner spawn is not tracked. If the outer task panics before reaching this point, the notification never fires.

**Recommendation**: Use `tokio::time::sleep` directly or handle the delay as part of the outer task's cleanup.

### Issue 3: `RwLock` Granularity in `AgentState`

`AgentState` wraps all fields in a single `RwLock<AgentInner>`:
```rust
pub struct AgentState {
    inner: RwLock<AgentInner>,
}
```

Each read of `status()`, `last_session_id()`, or `last_error()` acquires a read lock. Each write of `set_running()`, `set_idle()`, etc., acquires a write lock on the entire inner state.

**Assessment**: This is actually fine for the current use case — the lock is held briefly for simple reads/writes. However, if `pending_prompt` operations become more complex, consider splitting into separate locks.

### Issue 4: `async_trait` Should Be Replaced (Rust 1.75+)

The codebase uses `async_trait` in 23 files. Since Rust 1.75 (stable Dec 2023), native async functions in traits are supported:

```rust
// Current (with async_trait):
#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn find_by_id(&self, task_id: &str) -> Result<Option<Task>, sqlx::Error>;
}

// Idiomatic Rust 1.75+:
pub trait TaskRepository: Send + Sync {
    fn find_by_id(&self, task_id: &str) -> impl Future<Output = Result<Option<Task>, sqlx::Error>> + Send;
}
```

**Caveat**: Native async trait fns don't support `dyn` dispatch directly. Since the codebase uses `Arc<dyn TaskRepository>`, `async_trait` is still needed unless you switch to a different dispatch pattern (e.g., `trait_variant` or manual boxing).

**Recommendation**: Keep `async_trait` for now since `dyn` dispatch is essential for the repository pattern. Revisit when `dyn`-compatible async trait fns are stabilized.

---

## Database Access Patterns

### Strengths
- Parameterized queries throughout — no SQL injection risk
- `query_as` used for type-safe row mapping
- `fetch_optional` used correctly for nullable queries
- Connection pool shared via `Arc` (through `PgPool` which is already `Arc`-wrapped internally)

### Issue 1: Dynamic SQL in `update_task` (Critical)

The `update_task` method in `task/dao.rs` constructs SQL dynamically using `format!()`:

```rust
async fn update_task(&self, task_id: &str, patch: &TaskPatch) -> Result<(), sqlx::Error> {
    let mut sets = Vec::new();
    let mut idx = 2u32;

    macro_rules! maybe_set {
        ($field:ident, $col:expr) => {
            if patch.$field.is_some() {
                sets.push(format!("{} = ${}", $col, idx));  // Dynamic SQL!
                idx += 1;
            }
        };
    }
    // ...
    let sql = format!("UPDATE tasks SET {} WHERE task_id = $1", sets.join(", "));
    let mut query = sqlx::query(&sql).bind(task_id);
}
```

**Problems**:
1. Bypasses SQLx compile-time query checking
2. Column names come from string literals in macros, so typos won't be caught until runtime
3. The binding order must exactly match the `maybe_set!` → `maybe_bind!` order — a subtle invariant enforced only by code convention

**Recommendation**: Use individual UPDATE statements per field, or use a query builder like `sea-query`:
```rust
// Option A: Simple conditional updates (idiomatic for small patch sets)
if let Some(ref status) = patch.status {
    sqlx::query("UPDATE tasks SET status = $2 WHERE task_id = $1")
        .bind(task_id)
        .bind(status)
        .execute(&self.0)
        .await?;
}
if let Some(ref name) = patch.name {
    sqlx::query("UPDATE tasks SET name = $2 WHERE task_id = $1")
        .bind(task_id)
        .bind(name)
        .execute(&self.0)
        .await?;
}

// Option B: Use sea-query for type-safe dynamic query building
```

### Issue 2: N+1 Query Pattern in Task Listing

In `task/handlers.rs`, `resolve_user_display_names` and `resolve_repo_names` execute one query per unique user/repo:

```rust
async fn resolve_user_display_names(state: &AppState, user_ids: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for uid in user_ids {
        if let Some(name) = resolve_single_user_display_name(state, uid).await {
            map.insert(uid.clone(), name);
        }
    }
    map
}
```

For a page of 50 tasks with 20 unique creators and 10 unique repos, this is 30 extra queries.

**Recommendation**: Add batch query methods to the repository traits:
```rust
// In UserRepository:
async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, sqlx::Error>;

// Implementation:
sqlx::query_as("SELECT * FROM users WHERE user_id = ANY($1)")
    .bind(user_ids)
    .fetch_all(&self.0)
    .await
```

### Issue 3: Duplicate Count Query in `list_tasks`

`list_tasks` runs two nearly identical queries — one for data and one for count:
```rust
let rows = sqlx::query_as::<_, Task>(/* SELECT ... LIMIT $6 OFFSET $7 */)...;
let total: (i64,) = sqlx::query_as(/* SELECT COUNT(DISTINCT ...) same WHERE */)...;
```

**Recommendation**: Use a CTE or window function:
```sql
SELECT *, COUNT(*) OVER() AS total_count FROM tasks t ...
```
This eliminates the second query entirely.

### Issue 4: Connection Pool Size

`db.rs` hardcodes `max_connections(5)`:
```rust
pub async fn create_pool(database_url: &str) -> Result<DbPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await
}
```

**Recommendation**: Make this configurable via environment variable, and add connection timeouts:
```rust
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<DbPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .connect(database_url)
        .await
}
```

---

## Resource Management

### Issue 1: Configuration Has No Validation

`Config::from_env()` loads values from environment variables but performs no validation:
```rust
pub fn from_env() -> Self {
    let _ = dotenvy::dotenv();
    Self {
        server: ServerConfig {
            port: env_or("PORT", 8080),
            // No validation that port is in valid range
        },
        sandbox: SandboxConfig {
            namespace: env_str("SANDBOX_NAMESPACE", "workspaces"),
            // No validation that namespace is a valid K8s name
        },
        // ...
    }
}
```

**Recommendation**: Add a `validate()` method called after construction:
```rust
impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let config = Self { /* ... current code ... */ };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.server.port == 0 {
            return Err(ConfigError::Invalid("PORT cannot be 0".into()));
        }
        if self.sandbox.namespace.is_empty() {
            return Err(ConfigError::Invalid("SANDBOX_NAMESPACE cannot be empty".into()));
        }
        // Validate auth config consistency
        if let Some(AuthProvider::GitHub) = self.auth.provider {
            if self.auth.github_client_id.is_none() {
                return Err(ConfigError::Invalid("GITHUB_CLIENT_ID required when AUTH_PROVIDER=github".into()));
            }
        }
        Ok(())
    }
}
```

### Issue 2: Secrets Potentially Logged via `Debug`

`Config` derives `Debug`, meaning all fields (including secrets) will appear in debug output:
```rust
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub github_client_secret: Option<String>,  // Secret!
    pub auth0_client_secret: Option<String>,    // Secret!
    // ...
}
```

**Recommendation**: Implement custom `Debug` for config types containing secrets:
```rust
impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("provider", &self.provider)
            .field("github_client_id", &self.github_client_id)
            .field("github_client_secret", &self.github_client_secret.as_ref().map(|_| "[REDACTED]"))
            // ...
            .finish()
    }
}
```

Or use a `Secret<String>` wrapper type that redacts on `Debug`/`Display`.

### Issue 3: `env_or` Silently Swallows Parse Errors

```rust
fn env_or<T: FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())  // Parse error silently ignored!
        .unwrap_or(default)
}
```

If someone sets `PORT=abc`, it silently falls back to `8080` with no warning.

**Recommendation**: Log a warning on parse failure:
```rust
fn env_or<T: FromStr>(key: &str, default: T) -> T {
    match env::var(key) {
        Ok(v) => match v.parse() {
            Ok(parsed) => parsed,
            Err(_) => {
                tracing::warn!(key, value = %v, "Failed to parse env var, using default");
                default
            }
        },
        Err(_) => default,
    }
}
```

---

## Code Organization & Modularity

### Issue 1: Module Visibility is Too Broad

Many struct fields and functions are `pub` when they should be `pub(crate)`:
- All `AppState` fields are `pub` but are only accessed within the crate
- `K8sSandboxProvider` fields are all `pub` but constructed only in `startup.rs`
- `SandboxServiceImpl` in the sandbox has `pub state` — exposed wider than necessary

**Recommendation**: Default to `pub(crate)` for types only used within the crate, and `pub` only for types that form the public API.

### Issue 2: `#[allow(dead_code)]` and `#[allow(unused_assignments)]` Suppressions

Found several warning suppressions that may indicate issues:
- `task/models.rs:140` — `#[allow(dead_code)]` on `Task` struct
- `task/dao.rs:167` — `#[allow(unused_assignments)]` on `update_task`
- `extractors.rs:21,130` — `#[allow(dead_code)]` on `AuthUser` and `MaybeAuthUser`
- `sandbox/error.rs:9` — `#[allow(dead_code)]` on `SandboxError`

**Recommendation**: Remove dead code or use `pub(crate)` to make the compiler see the usage.

### Issue 3: Excessive Cloning

392 `.clone()` calls across 49 backend files. While many are necessary (cloning `Arc`, `String` for ownership transfer), some patterns suggest unnecessary cloning:

```rust
// task/handlers.rs — cloning task_id multiple times
let task_id = uuid::Uuid::new_v4().to_string();
// ... later:
tracing::info!(task_id = %task_id, user_id = %auth.user_id, repository_id = %req.repository_id, "Task created");
// ... and:
let task_response = enrich_task(&row, &state, &auth.user_id).await?;
Ok(Json(CreateTaskResponse {
    task_id: row.task_id.clone(),  // Clone from row, but we already have task_id
    // ...
}))
```

**Recommendation**: Audit clone sites, use references where possible, and consider `Cow<'_, str>` for functions that sometimes own and sometimes borrow.

---

## Testing Patterns

### Strengths
- `mockall::automock` on all repository traits — excellent for unit testing
- Serde round-trip tests for enum types
- Integration test directories present

### Issue 1: No Handler Unit Tests

Despite having `mockall` on all traits, there are no unit tests for handler functions (the core business logic). The `#[cfg(test)]` modules in model files only test serialization.

**Recommendation**: Add handler unit tests using mocked repositories:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::dao::MockTaskRepository;

    #[tokio::test]
    async fn create_task_validates_repository_exists() {
        let mut mock_repos = MockRepoRepository::new();
        mock_repos.expect_find_repository_by_id()
            .returning(|_| Ok(None)); // Repo doesn't exist

        let state = test_app_state(mock_repos);
        let result = create_task(State(state), auth, Json(req)).await;
        assert!(matches!(result, Err(TaskError::RepoNotFound(_))));
    }
}
```

### Issue 2: No Tests for Error Paths

The `IntoResponse` implementations for error types are untested — ensuring correct HTTP status codes is critical for API correctness.

---

## Security Considerations

### Issue 1: Internal Error Messages Leak to Clients

`SandboxError::IntoResponse` exposes internal error details:
```rust
Self::KubeError(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
```
This will include Kubernetes API error details in the HTTP response. Similarly, `TaskError::Internal` passes the message through directly.

**Recommendation**: Never expose internal error details. Return generic messages to clients, log the details server-side:
```rust
Self::KubeError(e) => {
    tracing::error!(error = %e, "Kubernetes API error");
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".to_string())
}
```

### Issue 2: Glob Pattern Injection in File Search

In `sandbox/src/grpc_server.rs`:
```rust
let full_pattern = format!("{workspace_dir}/{}", req.query);
let files: Vec<String> = glob::glob(&full_pattern)...;
```

If `req.query` contains `../../etc/passwd` or `**/*`, it could access files outside the workspace directory.

**Recommendation**: Validate and sanitize the query, and verify results are within the workspace:
```rust
// Reject path traversal
if req.query.contains("..") {
    return Err(Status::invalid_argument("Query must not contain '..'"));
}
// After resolving: verify paths are within workspace_dir
.filter_map(|p| {
    let canonical = p.canonicalize().ok()?;
    if !canonical.starts_with(workspace_dir) {
        return None; // Path traversal attempt
    }
    // ...
})
```

---

## Prioritized Recommendations

### P0 — Critical (Fix Now)
1. **Validate glob search input** in `sandbox/grpc_server.rs` to prevent path traversal
2. **Stop leaking internal errors** — audit all `IntoResponse` impls, return generic 500 messages for internal errors
3. **Add error handling to spawned critical tasks** — metrics server, gRPC server in `startup.rs`

### P1 — Important (Fix Soon)
4. **Fix N+1 queries** in `resolve_user_display_names` and `resolve_repo_names` — add batch methods
5. **Replace dynamic SQL** in `update_task` with type-safe alternatives
6. **Redact secrets in Debug output** — custom Debug impl or `Secret<T>` wrapper for config types
7. **Add config validation** at startup to fail fast on misconfiguration
8. **Log warnings on parse failures** in `env_or` instead of silently falling back

### P2 — Important (Plan for Next Sprint)
9. **Introduce `TaskId`/`UserId` newtypes** — start with `TaskId`, most impactful
10. **Use `DateTime<Utc>` for timestamps** instead of `String`
11. **Replace manual sqlx Type impls** with `#[derive(sqlx::Type)]` — ~180 lines removed
12. **Add handler unit tests** using mockall
13. **Extract shared task creation logic** from `create_task`/`create_vibe_task`
14. **Use `TaskFilter` struct** to replace 7-parameter `list_tasks`
15. **Make DB pool size configurable** via env var

### P3 — Nice to Have (Incremental)
16. **Replace `Box<dyn Error>`** with typed errors in `SandboxGrpcClient`, `BackendClient`, internal consumers
17. **Add `Default` impl and constructors** for `TaskMessage` to reduce 30-field construction
18. **Reduce `pub` to `pub(crate)`** across structs used only internally
19. **Remove `#[allow(dead_code)]`** — investigate if code is actually unused
20. **Use CTE with window function** for list_tasks to eliminate duplicate count query
21. **Audit `.clone()` sites** — reduce unnecessary cloning where references suffice
22. **Store `JoinHandle`s** for background tasks and integrate with shutdown coordination
