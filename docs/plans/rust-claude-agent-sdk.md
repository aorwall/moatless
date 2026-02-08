# Plan: Rust Claude Agent SDK Crate

## Context

We need a Rust SDK for the Claude Code CLI, following the same pattern as the official [Python SDK](https://github.com/anthropics/claude-agent-sdk-python) and [TypeScript SDK](https://platform.claude.com/docs/en/agent-sdk/typescript). Both official SDKs work by spawning the `claude` CLI as a subprocess with `--output-format stream-json` and communicating via a bidirectional NDJSON control protocol over stdin/stdout. This Rust crate will enable the sandbox-controller (and future Rust services) to execute Claude agents natively without the Node.js sandbox app.

The crate is designed as a standalone, publishable library with no moatless-specific code.

## How the Official SDKs Work

Both the TypeScript and Python SDKs follow the same architecture:

1. **Spawn `claude` CLI** as a child process with `--output-format stream-json --input-format stream-json --verbose`
2. **Read NDJSON from stdout** — each line is a JSON object with a `type` field discriminating between SDK messages (`assistant`, `user`, `system`, `result`, `stream_event`) and control protocol messages (`control_request`, `control_response`)
3. **Write NDJSON to stdin** — user messages, control requests (initialize, interrupt), and control responses (permission results, hook outputs, MCP responses)
4. **Bidirectional control protocol** — the CLI can ask the SDK for permission to use tools (`can_use_tool`), trigger registered hooks (`hook_callback`), or forward MCP tool calls (`mcp_message`). Each request has a unique `request_id` for response correlation.
5. **Capture stderr** — for diagnostics and debugging

### Key Insight: SDK ≠ API Client

The SDK is **not** a wrapper around the Anthropic API. It wraps the Claude Code **CLI application**, which provides:
- Built-in tools (Read, Write, Edit, Bash, Glob, Grep, WebFetch, WebSearch, etc.)
- File checkpointing and session management
- MCP server orchestration
- Subagent spawning (Task tool)
- The full Claude Code system prompt and behavior

## Architecture Overview

```
┌─────────────────────────────────────────────┐
│  Rust SDK (claude-agent-sdk)                │
│                                              │
│  ┌──────────────┐    ┌───────────────────┐  │
│  │ query() /     │    │ SubprocessTransport│  │
│  │ ClaudeClient  │───►│                   │  │
│  └──────┬───────┘    │  stdin  ──► claude │  │
│         │            │  stdout ◄── claude │  │
│         ▼            │  stderr ◄── claude │  │
│  ┌──────────────┐    └───────────────────┘  │
│  │ QueryOrch.   │                            │
│  │  - reader    │──► mpsc::Sender<Message>   │
│  │  - tracker   │                            │
│  │  - handlers  │──► can_use_tool / hooks    │
│  └──────────────┘                            │
│         │                                    │
│         ▼                                    │
│  ReceiverStream<Result<Message>>             │
└─────────────────────────────────────────────┘
```

**Reader task** (tokio::spawn): Reads stdout line-by-line, classifies each JSON object:
- `control_response` → resolves pending oneshot channel in RequestTracker
- `control_request` → spawns handler task (permission/hook/MCP), writes response to stdin
- Any other `type` → forwards as `Message` through mpsc channel to consumer

**Consumer**: Receives `Message` objects as a `tokio_stream::Stream`, exposed via `query()` or `ClaudeClient`.

## Crate Location

```
crates/claude-agent-sdk/
```

Added to workspace `Cargo.toml` members alongside `sandbox-controller` and `crates/figma-clipboard`.

## Module Layout

```
crates/claude-agent-sdk/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # Public re-exports
│   ├── error.rs                # SdkError enum (thiserror)
│   ├── query.rs                # Public query() + query_streaming() functions
│   ├── client.rs               # ClaudeClient for multi-turn sessions
│   ├── types/
│   │   ├── mod.rs
│   │   ├── messages.rs         # Message enum + all message variants
│   │   ├── content.rs          # TextBlock, ToolUseBlock, ToolResultBlock, ThinkingBlock
│   │   ├── options.rs          # ClaudeOptions, SystemPrompt, PermissionMode, etc.
│   │   ├── control.rs          # Control protocol request/response types
│   │   ├── permissions.rs      # PermissionHandler trait, PermissionResult
│   │   ├── hooks.rs            # HookHandler trait, HookEvent, HookMatcher
│   │   ├── mcp.rs              # McpServerConfig variants
│   │   └── agents.rs           # AgentDefinition
│   ├── transport/
│   │   ├── mod.rs              # Transport trait
│   │   └── subprocess.rs       # SubprocessTransport (spawns claude CLI)
│   └── protocol/
│       ├── mod.rs
│       ├── orchestrator.rs     # Reader task, message routing, control dispatch
│       └── request_tracker.rs  # Request ID correlation with oneshot channels
├── tests/
│   ├── fixtures/               # Captured NDJSON from real claude runs
│   ├── message_parsing.rs      # Serde round-trip tests
│   ├── command_building.rs     # ClaudeOptions → CLI flags tests
│   └── protocol_tests.rs       # Mock transport control protocol tests
```

## Dependencies

```toml
[package]
name = "claude-agent-sdk"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "Rust SDK for the Claude Code CLI agent"
keywords = ["claude", "ai", "agent", "sdk", "cli"]
categories = ["api-bindings", "asynchronous"]

[dependencies]
tokio = { version = "1", features = ["process", "io-util", "sync", "macros", "rt"] }
tokio-stream = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
uuid = { version = "1", features = ["v4"] }
async-trait = "0.1"
futures = "0.3"

[dev-dependencies]
tokio = { version = "1", features = ["full", "test-util"] }
pretty_assertions = "1"
```

## Detailed Type Definitions

### Message Types (from CLI stdout NDJSON)

Each line from stdout is a JSON object with a `type` field:

```rust
/// Top-level message from CLI stdout. Discriminated by "type" field.
enum Message {
    Assistant(AssistantMessage),  // Claude's response with content blocks
    User(UserMessage),           // Echoed user messages / tool results
    System(SystemMessage),       // Init, status, compact_boundary, hooks
    Result(ResultMessage),       // Final result with cost/usage/session_id
    StreamEvent(StreamEvent),    // Partial streaming events (if enabled)
}
```

**Serde strategy**: Deserialize to `serde_json::Value` first, inspect `"type"`, then deserialize the inner variant. This avoids issues with two-level tag discrimination (`type` + `subtype` for SystemMessage).

#### AssistantMessage

```rust
struct AssistantMessage {
    uuid: String,
    session_id: String,
    message: ApiMessage,           // { role: "assistant", content: Vec<ContentBlock> }
    parent_tool_use_id: Option<String>,
    error: Option<String>,
}
```

#### UserMessage

```rust
struct UserMessage {
    uuid: Option<String>,
    session_id: String,
    message: ApiMessage,           // { role: "user", content: ... }
    parent_tool_use_id: Option<String>,
    tool_use_result: Option<Value>,
}
```

#### SystemMessage (discriminated by `subtype`)

```rust
enum SystemMessage {
    Init {
        uuid: String,
        session_id: String,
        tools: Vec<String>,
        model: String,
        mcp_servers: Vec<McpServerInfo>,
        permission_mode: String,
        cwd: String,
    },
    Status { /* ... */ },
    CompactBoundary {
        uuid: String,
        session_id: String,
        compact_metadata: CompactMetadata,
    },
    HookStarted { /* ... */ },
    HookProgress { /* ... */ },
    HookResponse { /* ... */ },
}
```

#### ResultMessage

```rust
struct ResultMessage {
    subtype: String,          // "success" | "error_max_turns" | "error_during_execution" | ...
    uuid: String,
    session_id: String,
    duration_ms: u64,
    duration_api_ms: u64,
    is_error: bool,
    num_turns: u32,
    total_cost_usd: f64,
    result: Option<String>,   // text result on success
    usage: Option<Value>,
    model_usage: Option<Value>,
    structured_output: Option<Value>,
}
```

### Content Blocks

```rust
enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String, signature: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { tool_use_id: String, content: Option<Value>, is_error: Option<bool> },
}
```

### ClaudeOptions

```rust
#[derive(Debug, Clone, Default)]
struct ClaudeOptions {
    // Model
    model: Option<String>,                      // --model
    fallback_model: Option<String>,             // --fallback-model

    // System prompt
    system_prompt: Option<SystemPrompt>,        // --system-prompt or --append-system-prompt

    // Permissions
    permission_mode: Option<PermissionMode>,    // --permission-mode
    permission_handler: Option<Arc<dyn PermissionHandler>>,

    // Tools
    tools: Option<ToolsConfig>,                 // --tools
    allowed_tools: Option<Vec<String>>,         // --allowedTools
    disallowed_tools: Option<Vec<String>>,      // --disallowedTools

    // Limits
    max_turns: Option<u32>,                     // --max-turns
    max_budget_usd: Option<f64>,                // --max-budget-usd
    max_thinking_tokens: Option<u32>,           // via initialize

    // Session
    resume: Option<String>,                     // --resume
    continue_conversation: bool,                // --continue
    fork_session: bool,                         // --fork-session
    session_id: Option<String>,                 // --session-id

    // Directories
    cwd: Option<PathBuf>,                       // subprocess working directory
    add_dirs: Option<Vec<PathBuf>>,             // --add-dir

    // MCP & Plugins
    mcp_servers: Option<HashMap<String, McpServerConfig>>,  // --mcp-config
    plugins: Option<Vec<PluginConfig>>,         // --plugin-dir

    // Agents
    agents: Option<HashMap<String, AgentDefinition>>,  // via initialize request
    agent: Option<String>,                      // --agent (main thread agent)

    // Settings
    setting_sources: Option<Vec<SettingSource>>,// --setting-sources
    sandbox: Option<SandboxSettings>,           // merged into --settings

    // Streaming
    include_partial_messages: bool,             // --include-partial-messages
    betas: Option<Vec<String>>,                 // --betas

    // Structured output
    output_format: Option<OutputFormat>,        // --json-schema

    // Hooks
    hooks: Option<HashMap<HookEvent, Vec<HookMatcher>>>,

    // Process
    cli_path: Option<PathBuf>,                  // path to claude binary
    env: Option<HashMap<String, String>>,       // subprocess env vars
    extra_args: Option<HashMap<String, Option<String>>>,  // arbitrary --flags

    // Callbacks
    stderr_callback: Option<Arc<dyn Fn(String) + Send + Sync>>,
}
```

#### ClaudeOptions → CLI Flags Mapping

| Option | CLI Flag | Notes |
|--------|----------|-------|
| `model` | `--model` | |
| `system_prompt` (Replace) | `--system-prompt "..."` | |
| `system_prompt` (Preset+Append) | `--append-system-prompt "..."` | |
| `permission_mode` | `--permission-mode` | default/acceptEdits/plan/bypassPermissions |
| `allowed_tools` | `--allowedTools` | comma-separated |
| `disallowed_tools` | `--disallowedTools` | comma-separated |
| `tools` | `--tools` | "default" or comma-separated names |
| `max_turns` | `--max-turns` | |
| `max_budget_usd` | `--max-budget-usd` | |
| `resume` | `--resume` | session ID string |
| `continue_conversation` | `--continue` | |
| `fork_session` | `--fork-session` | |
| `session_id` | `--session-id` | UUID |
| `add_dirs` | `--add-dir` (repeated) | |
| `mcp_servers` | `--mcp-config` | JSON string or file path |
| `agents` | via `initialize` control request | not a CLI flag |
| `setting_sources` | `--setting-sources` | comma-separated |
| `fallback_model` | `--fallback-model` | |
| `betas` | `--betas` | comma-separated |
| `include_partial_messages` | `--include-partial-messages` | |
| `plugins` | `--plugin-dir` (repeated) | |
| `output_format` | `--json-schema` | JSON schema string |
| `cli_path` | subprocess executable path | not a flag |
| `cwd` | subprocess working directory | not a flag |
| `env` | subprocess environment | not a flag |

### SystemPrompt Enum

```rust
enum SystemPrompt {
    /// Replace the entire system prompt
    Replace(String),
    /// Use Claude Code's built-in prompt with optional appended text
    Preset { append: Option<String> },
}
```

### PermissionMode Enum

```rust
enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    BypassPermissions,
}
```

### Control Protocol Types

#### Outgoing (SDK → CLI, written to stdin)

```rust
// Wrapper sent to stdin
struct ControlRequest {
    type: "control_request",
    request_id: String,
    request: ControlRequestInner,
}

enum ControlRequestInner {
    Initialize {
        hooks: Option<Value>,       // hook matchers with callback IDs
        agents: Option<Value>,      // agent definitions
    },
    Interrupt,
    SetPermissionMode { mode: String },
    SetModel { model: Option<String> },
    SetMaxThinkingTokens { max_thinking_tokens: Option<u32> },
    RewindFiles { user_message_id: String },
}
```

#### Incoming (CLI → SDK, from stdout)

```rust
// Incoming control request from CLI
struct IncomingControlRequest {
    request_id: String,
    request: IncomingControlRequestInner,
}

enum IncomingControlRequestInner {
    CanUseTool {
        tool_name: String,
        input: Value,
        tool_use_id: String,
        permission_suggestions: Option<Vec<Value>>,
        blocked_path: Option<String>,
        agent_id: Option<String>,
    },
    HookCallback {
        callback_id: String,
        input: Value,
        tool_use_id: Option<String>,
    },
    McpMessage {
        server_name: String,
        message: Value,
    },
}
```

#### Control Responses (bidirectional)

```rust
struct ControlResponse {
    type: "control_response",
    response: ControlResponseInner,
}

enum ControlResponseInner {
    Success { request_id: String, response: Option<Value> },
    Error { request_id: String, error: String },
}
```

### PermissionHandler Trait

```rust
#[async_trait]
pub trait PermissionHandler: Send + Sync {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        context: ToolPermissionContext,
    ) -> PermissionResult;
}

struct ToolPermissionContext {
    suggestions: Vec<Value>,
    blocked_path: Option<String>,
    tool_use_id: String,
    agent_id: Option<String>,
}

enum PermissionResult {
    Allow {
        updated_input: Option<Value>,
        updated_permissions: Option<Vec<Value>>,
    },
    Deny {
        message: String,
        interrupt: bool,
    },
}
```

Stored as `Option<Arc<dyn PermissionHandler>>` in the orchestrator. When the CLI sends a `can_use_tool` control request, a tokio task is spawned that:
1. Clones the `Arc<dyn PermissionHandler>`
2. Calls `can_use_tool()` on it
3. Serializes the `PermissionResult` to a control response
4. Writes the response to stdin (mutex-protected)

A convenience `AllowAllPermissions` struct is provided for common use.

### HookHandler Trait

```rust
#[async_trait]
pub trait HookHandler: Send + Sync {
    async fn on_hook(
        &self,
        input: &serde_json::Value,
        tool_use_id: Option<&str>,
    ) -> serde_json::Value; // Returns HookJSONOutput
}
```

### MCP Server Config

```rust
enum McpServerConfig {
    Stdio {
        command: String,
        args: Option<Vec<String>>,
        env: Option<HashMap<String, String>>,
    },
    Sse {
        url: String,
        headers: Option<HashMap<String, String>>,
    },
    Http {
        url: String,
        headers: Option<HashMap<String, String>>,
    },
}
```

### AgentDefinition

```rust
struct AgentDefinition {
    description: String,
    prompt: String,
    tools: Option<Vec<String>>,
    model: Option<AgentModel>,  // "sonnet" | "opus" | "haiku" | "inherit"
}
```

## Error Types

```rust
#[derive(Error, Debug)]
pub enum SdkError {
    #[error("Claude CLI not found at: {path}")]
    CliNotFound { path: String },

    #[error("Failed to spawn CLI process: {source}")]
    ProcessSpawn { #[source] source: std::io::Error },

    #[error("CLI process exited with code {exit_code}: {stderr}")]
    ProcessFailed { exit_code: i32, stderr: String },

    #[error("Failed to parse NDJSON: {source}")]
    JsonDecode { #[source] source: serde_json::Error, raw_line: String },

    #[error("Control request timed out ({timeout_secs}s) for: {subtype}")]
    ControlTimeout { timeout_secs: u64, subtype: String },

    #[error("Control request error: {message}")]
    ControlError { request_id: String, message: String },

    #[error("Transport not connected")]
    NotConnected,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Stdin write failed: {source}")]
    StdinWrite { #[source] source: std::io::Error },

    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },

    #[error("No permission handler configured")]
    NoPermissionHandler,

    #[error("Hook callback not found: {callback_id}")]
    HookCallbackNotFound { callback_id: String },
}
```

## Public API

### `query()` — One-shot Execution

```rust
/// Execute a one-shot query against the Claude CLI.
///
/// Returns an async stream of messages. The stream completes when the
/// CLI process exits (after emitting a Result message).
///
/// # Example
/// ```rust
/// use claude_agent_sdk::{query, ClaudeOptions, PermissionMode};
/// use tokio_stream::StreamExt;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let options = ClaudeOptions {
///         permission_mode: Some(PermissionMode::AcceptEdits),
///         max_turns: Some(3),
///         cwd: Some("/my/project".into()),
///         ..Default::default()
///     };
///
///     let mut stream = query("Add a README.md file", options).await?;
///
///     while let Some(msg) = stream.next().await {
///         match msg? {
///             Message::Assistant(a) => println!("Claude: {:?}", a.message),
///             Message::Result(r) => println!("Done! Cost: ${}", r.total_cost_usd),
///             _ => {}
///         }
///     }
///     Ok(())
/// }
/// ```
pub async fn query(
    prompt: &str,
    options: ClaudeOptions,
) -> Result<impl Stream<Item = Result<Message>>>
```

**Internal flow:**
1. Build CLI command from `ClaudeOptions`
2. Spawn subprocess via `SubprocessTransport`
3. Create `QueryOrchestrator` (starts reader task, initializes request tracker)
4. Send `initialize` control request with hooks/agents
5. Write user message to stdin as NDJSON, then close stdin
6. Return `ReceiverStream<Result<Message>>`

### `query_streaming()` — Multi-turn with Input Stream

```rust
/// Execute a streaming query with bidirectional message input.
///
/// Accepts an async stream of user messages, enabling multi-turn
/// conversations. Stdin stays open; messages are written as they arrive.
pub async fn query_streaming<S>(
    input: S,
    options: ClaudeOptions,
) -> Result<impl Stream<Item = Result<Message>>>
where
    S: Stream<Item = UserMessage> + Send + 'static,
```

Same as `query()` but:
- Does **not** close stdin after first message
- Spawns an input streamer task that reads from `S` and writes NDJSON to stdin
- Stdin remains open for control protocol traffic

### `ClaudeClient` — Multi-turn Session Manager

```rust
/// Client for bidirectional, interactive conversations.
///
/// Provides send/receive interface with session management.
///
/// # Example
/// ```rust
/// use claude_agent_sdk::{ClaudeClient, ClaudeOptions, PermissionMode};
/// use tokio_stream::StreamExt;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let options = ClaudeOptions {
///         permission_mode: Some(PermissionMode::AcceptEdits),
///         cwd: Some("/my/project".into()),
///         ..Default::default()
///     };
///
///     let mut client = ClaudeClient::new(options).await?;
///
///     // First turn
///     client.send("List all TODO comments in the codebase").await?;
///     while let Some(msg) = client.messages().next().await {
///         println!("{:?}", msg?);
///     }
///
///     // Second turn (same session)
///     client.send("Now fix the most critical one").await?;
///     while let Some(msg) = client.messages().next().await {
///         println!("{:?}", msg?);
///     }
///
///     client.close().await;
///     Ok(())
/// }
/// ```
pub struct ClaudeClient { /* internal state */ }

impl ClaudeClient {
    pub async fn new(options: ClaudeOptions) -> Result<Self>;
    pub async fn send(&self, message: &str) -> Result<()>;
    pub fn messages(&self) -> impl Stream<Item = Result<Message>> + '_;
    pub async fn interrupt(&self) -> Result<()>;
    pub async fn set_model(&self, model: Option<&str>) -> Result<()>;
    pub async fn set_permission_mode(&self, mode: PermissionMode) -> Result<()>;
    pub fn session_id(&self) -> Option<&str>;
    pub async fn close(self);
}
```

### `lib.rs` Re-exports

```rust
pub mod types;
pub mod transport;
pub mod protocol;
pub mod error;
pub mod client;
pub mod query;

// Top-level re-exports for ergonomic use
pub use error::{SdkError, Result};
pub use types::messages::Message;
pub use types::content::ContentBlock;
pub use types::options::{ClaudeOptions, SystemPrompt, PermissionMode};
pub use types::permissions::{PermissionHandler, PermissionResult, AllowAllPermissions};
pub use types::hooks::{HookHandler, HookEvent, HookMatcher};
pub use types::agents::AgentDefinition;
pub use types::mcp::McpServerConfig;
pub use query::{query, query_streaming};
pub use client::ClaudeClient;
```

## Transport Layer Detail

### Transport Trait

```rust
#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&mut self) -> Result<()>;
    async fn write(&self, data: &str) -> Result<()>;
    async fn end_input(&self) -> Result<()>;
    fn close(&self);
    fn is_ready(&self) -> bool;
}
```

### SubprocessTransport

```rust
struct SubprocessTransport {
    options: ClaudeOptions,
    child: Option<Child>,
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    stdout: Option<BufReader<ChildStdout>>,
    stderr_task: Option<JoinHandle<()>>,
}

impl SubprocessTransport {
    fn build_command(&self) -> Vec<String>;  // ClaudeOptions → CLI args
    fn find_cli(&self) -> Result<PathBuf>;   // Locate claude binary
}
```

**CLI binary discovery order:**
1. `options.cli_path` if set
2. `which claude` (PATH lookup)
3. `~/.claude/local/claude`
4. Common npm global locations

**Process management:**
- `close()` sends SIGTERM, then SIGKILL after 5s timeout
- Stderr is drained by a background task, lines logged via `tracing::debug!`
- If `stderr_callback` is set, lines are also forwarded to the callback

## Protocol Layer Detail

### RequestTracker

```rust
struct RequestTracker {
    counter: u64,
    pending: HashMap<String, oneshot::Sender<Result<Value, String>>>,
}

impl RequestTracker {
    fn create_request(&mut self) -> (String, oneshot::Receiver<...>);
    fn resolve(&mut self, request_id: &str, result: Result<Value, String>) -> bool;
    fn fail_all(&mut self, error: &str);
}
```

Generates unique request IDs (`req_{counter}_{uuid}`), stores oneshot senders, resolves them when control responses arrive.

### QueryOrchestrator

The core message router. Runs as follows:

```
┌─────────────────────────────────────────┐
│ QueryOrchestrator                       │
│                                         │
│  tokio::spawn(reader_task)              │
│    │                                    │
│    ├─ read stdout line-by-line          │
│    ├─ parse JSON, check "type"          │
│    │                                    │
│    ├─ "control_response"                │
│    │   └─ tracker.resolve(request_id)   │
│    │                                    │
│    ├─ "control_request"                 │
│    │   ├─ "can_use_tool"               │
│    │   │   └─ spawn(handler.can_use_tool)│
│    │   │       └─ write response stdin  │
│    │   ├─ "hook_callback"              │
│    │   │   └─ spawn(hook.on_hook)       │
│    │   │       └─ write response stdin  │
│    │   └─ "mcp_message"               │
│    │       └─ spawn(mcp_handler)        │
│    │           └─ write response stdin  │
│    │                                    │
│    └─ any other type                    │
│        └─ mpsc_tx.send(Message)         │
│                                         │
│  send_control_request(req)              │
│    ├─ tracker.create_request()          │
│    ├─ write JSON to stdin               │
│    └─ await oneshot rx (with timeout)   │
│                                         │
│  mpsc_rx ──► consumer stream            │
└─────────────────────────────────────────┘
```

**Concurrency considerations:**
- Stdin writes are protected by `Arc<Mutex<ChildStdin>>` — both control response handlers and the input streamer may write concurrently
- The reader task owns the stdout reader exclusively
- `RequestTracker` is protected by a `Mutex` (accessed by reader task for resolve, by orchestrator for create)
- On process exit, reader task calls `tracker.fail_all()` to unblock any pending requests

## Implementation Phases

### Phase 1: Types + Parsing

**Create crate scaffold:**
- `crates/claude-agent-sdk/Cargo.toml`
- Add `"crates/claude-agent-sdk"` to root `Cargo.toml` workspace members

**Implement types:**
- `src/types/content.rs` — ContentBlock enum with serde
- `src/types/messages.rs` — Message enum with custom deserialization
- `src/types/options.rs` — ClaudeOptions, SystemPrompt, PermissionMode, etc.
- `src/types/control.rs` — Control protocol request/response types
- `src/types/permissions.rs` — PermissionHandler trait, PermissionResult
- `src/types/hooks.rs` — HookHandler trait, HookEvent, HookMatcher
- `src/types/mcp.rs` — McpServerConfig
- `src/types/agents.rs` — AgentDefinition
- `src/error.rs` — SdkError enum

**Tests:** Serde round-trip tests with fixture JSON captured from real `claude` runs.

### Phase 2: Transport

- `src/transport/mod.rs` — Transport trait
- `src/transport/subprocess.rs` — SubprocessTransport
  - `build_command()` — ClaudeOptions → CLI flags
  - `find_cli()` — binary discovery
  - `connect()` — spawn process, set up pipes
  - `write()` — mutex-protected stdin writes
  - `end_input()` — close stdin
  - `close()` — SIGTERM → SIGKILL escalation
  - Stderr drain task

**Tests:** Unit tests for `build_command()` (no subprocess needed).

### Phase 3: Protocol Layer

- `src/protocol/request_tracker.rs` — RequestTracker with oneshot channels
- `src/protocol/orchestrator.rs` — QueryOrchestrator
  - Reader task with message classification
  - Control request dispatch (can_use_tool, hook_callback, mcp_message)
  - `send_control_request()` with timeout

**Tests:** Mock transport tests — feed predefined NDJSON, verify message routing and control flow.

### Phase 4: Public API

- `src/query.rs` — `query()` and `query_streaming()`
- `src/lib.rs` — public re-exports

**Tests:** Integration tests with real `claude` binary (`#[ignore]` gated).

### Phase 5: ClaudeClient

- `src/client.rs` — `ClaudeClient`
  - Multi-turn session management
  - `send()`, `messages()`, `interrupt()`
  - Control methods: `set_model()`, `set_permission_mode()`
  - `session_id()` accessor
  - `close()` with graceful shutdown

**Tests:** Integration tests with multi-turn conversations.

## Files to Create/Modify

| File | Action |
|------|--------|
| `Cargo.toml` (root) | Add `"crates/claude-agent-sdk"` to workspace members |
| `crates/claude-agent-sdk/Cargo.toml` | New — crate manifest |
| `crates/claude-agent-sdk/src/lib.rs` | New — public re-exports |
| `crates/claude-agent-sdk/src/error.rs` | New — SdkError enum |
| `crates/claude-agent-sdk/src/query.rs` | New — query() public API |
| `crates/claude-agent-sdk/src/client.rs` | New — ClaudeClient |
| `crates/claude-agent-sdk/src/types/mod.rs` | New — module declarations |
| `crates/claude-agent-sdk/src/types/messages.rs` | New — Message enum + variants |
| `crates/claude-agent-sdk/src/types/content.rs` | New — content blocks |
| `crates/claude-agent-sdk/src/types/options.rs` | New — ClaudeOptions |
| `crates/claude-agent-sdk/src/types/control.rs` | New — control protocol types |
| `crates/claude-agent-sdk/src/types/permissions.rs` | New — PermissionHandler trait |
| `crates/claude-agent-sdk/src/types/hooks.rs` | New — HookHandler trait |
| `crates/claude-agent-sdk/src/types/mcp.rs` | New — MCP config types |
| `crates/claude-agent-sdk/src/types/agents.rs` | New — AgentDefinition |
| `crates/claude-agent-sdk/src/transport/mod.rs` | New — Transport trait |
| `crates/claude-agent-sdk/src/transport/subprocess.rs` | New — SubprocessTransport |
| `crates/claude-agent-sdk/src/protocol/mod.rs` | New — module declarations |
| `crates/claude-agent-sdk/src/protocol/orchestrator.rs` | New — message router |
| `crates/claude-agent-sdk/src/protocol/request_tracker.rs` | New — request correlation |

## Verification

1. **Unit tests**: `cargo test -p claude-agent-sdk`
2. **Build**: `cargo build -p claude-agent-sdk`
3. **Clippy**: `cargo clippy -p claude-agent-sdk`
4. **Integration test** (manual, requires `claude` binary installed):
   ```rust
   #[tokio::test]
   #[ignore]
   async fn test_simple_query() {
       let options = ClaudeOptions {
           permission_mode: Some(PermissionMode::BypassPermissions),
           max_turns: Some(1),
           ..Default::default()
       };
       let mut stream = query("What is 2+2?", options).await.unwrap();
       let mut got_result = false;
       while let Some(msg) = stream.next().await {
           if matches!(msg.unwrap(), Message::Result(_)) {
               got_result = true;
           }
       }
       assert!(got_result);
   }
   ```

## Reference Materials

- **TypeScript SDK**: https://platform.claude.com/docs/en/agent-sdk/typescript
- **Python SDK**: https://github.com/anthropics/claude-agent-sdk-python
- **CLI Reference**: https://code.claude.com/docs/en/cli-reference
- **Python SDK `subprocess_cli.py`**: Transport implementation showing command building
- **Python SDK `query.py` (internal)**: Control protocol orchestration
- **Python SDK `types.py`**: Complete type definitions including control protocol
- **Python SDK `message_parser.py`**: NDJSON → typed message parsing
