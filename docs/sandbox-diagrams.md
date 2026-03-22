# Sandbox Architecture Diagrams

Visual documentation of the `moatless-sandbox` crate — the Rust runtime that executes inside each sandbox Kubernetes pod.

---

## 1. Module Dependency Graph

Internal `use crate::` relationships between all source files.

```mermaid
graph TD
    classDef root fill:#4a90d9,stroke:#2c5282,color:#fff
    classDef agent fill:#e07b39,stroke:#9c4221,color:#fff
    classDef init fill:#38a169,stroke:#276749,color:#fff
    classDef filesMod fill:#9f7aea,stroke:#6b46c1,color:#fff

    %% ── Root modules ──────────────────────────────────
    subgraph root["Root"]
        main["main.rs"]:::root
        lib["lib.rs"]:::root
        state["state.rs"]:::root
        config["config.rs"]:::root
        error["error.rs"]:::root
        events["events.rs"]:::root
        backend_client["backend_client.rs"]:::root
        grpc_server["grpc_server.rs"]:::root
    end

    %% ── Agent module ──────────────────────────────────
    subgraph agent["agent/"]
        agent_state["agent_state.rs"]:::agent
        claude_code["claude_code.rs"]:::agent
        consumer["consumer.rs"]:::agent
        options["options.rs"]:::agent
        permission_handler["permission_handler.rs"]:::agent
        prompt["prompt.rs"]:::agent
        types["types.rs"]:::agent
    end

    %% ── Init module ──────────────────────────────────
    subgraph initMod["init/"]
        init_mod["mod.rs"]:::init
        credentials["credentials.rs"]:::init
        dirs["dirs.rs"]:::init
        git["git.rs"]:::init
        workspace["workspace.rs"]:::init
    end

    %% ── Files module ─────────────────────────────────
    subgraph filesMod2["files/"]
        files_mod["mod.rs"]:::filesMod
        tree["tree.rs"]:::filesMod
        read_write["read_write.rs"]:::filesMod
        files_git["git.rs"]:::filesMod
        search["search.rs"]:::filesMod
    end

    %% ── main.rs edges ────────────────────────────────
    main --> state
    main --> config
    main --> backend_client
    main --> events
    main --> grpc_server
    main --> init_mod
    main --> agent_state

    %% ── state.rs edges ───────────────────────────────
    state --> config
    state --> backend_client
    state --> events
    state --> agent_state
    state --> init_mod

    %% ── events.rs edges ──────────────────────────────
    events --> backend_client

    %% ── grpc_server.rs edges ─────────────────────────
    grpc_server --> state
    grpc_server --> claude_code
    grpc_server --> consumer
    grpc_server --> types
    grpc_server --> tree
    grpc_server --> read_write
    grpc_server --> search
    grpc_server --> files_mod

    %% ── agent/claude_code.rs edges ───────────────────
    claude_code --> state
    claude_code --> error
    claude_code --> options
    claude_code --> types
    claude_code --> consumer
    claude_code --> credentials

    %% ── agent/consumer.rs edges ──────────────────────
    consumer --> state
    consumer --> claude_code
    consumer --> prompt
    consumer --> types

    %% ── agent/options.rs edges ───────────────────────
    options --> config
    options --> types
    options --> agent_state
    options --> permission_handler

    %% ── agent/permission_handler.rs edges ────────────
    permission_handler --> agent_state
    permission_handler --> prompt

    %% ── init/mod.rs edges ────────────────────────────
    init_mod --> state
    init_mod --> credentials
    init_mod --> dirs
    init_mod --> git
    init_mod --> workspace
    init_mod --> consumer

    %% ── init submodule edges ─────────────────────────
    credentials --> state
    dirs --> config
    git --> config
    workspace --> config

    %% ── files submodule edges ────────────────────────
    tree --> files_git
    search --> tree
```

---

## 2. Component & Communication Architecture

The sandbox pod as a system, showing external dependencies, shared crates, and communication channels.

```mermaid
graph LR
    classDef external fill:#718096,stroke:#4a5568,color:#fff
    classDef crate fill:#dd6b20,stroke:#9c4221,color:#fff
    classDef internal fill:#4a90d9,stroke:#2c5282,color:#fff

    %% ── External systems ─────────────────────────────
    Backend["Backend<br/>(Rust/Axum)"]:::external
    ClaudeCLI["Claude CLI<br/>(subprocess)"]:::external
    GitRepo["Git Repository"]:::external
    Filesystem["Filesystem<br/>(/workspace)"]:::external
    K8s["K8s Control Plane<br/>(health probes)"]:::external

    %% ── Sandbox pod ──────────────────────────────────
    subgraph pod["Sandbox Pod (per task)"]
        direction TB

        subgraph transport["Transport Layer"]
            GrpcServer["gRPC Server<br/><i>grpc_server.rs</i><br/>port 50052"]:::internal
            HttpServer["HTTP Server<br/><i>main.rs</i><br/>port 8001"]:::internal
            GrpcClient["gRPC Client<br/><i>backend_client.rs</i>"]:::internal
        end

        subgraph core["Core"]
            State["SandboxState<br/><i>state.rs</i>"]:::internal
            Events["GrpcEventPublisher<br/><i>events.rs</i>"]:::internal
            Config["Config<br/><i>config.rs</i>"]:::internal
        end

        subgraph agentMod["Agent Module"]
            Executor["Agent Executor<br/><i>claude_code.rs</i>"]:::internal
            Consumer["Message Consumer<br/><i>consumer.rs</i>"]:::internal
            AgentState["AgentState<br/><i>agent_state.rs</i>"]:::internal
            PermHandler["Permission Handler<br/><i>permission_handler.rs</i>"]:::internal
            Options["Options Builder<br/><i>options.rs</i>"]:::internal
        end

        subgraph initMod["Init Module"]
            InitOrch["Init Orchestrator<br/><i>init/mod.rs</i>"]:::internal
            Credentials["Credentials<br/><i>credentials.rs</i>"]:::internal
            GitInit["Git Init<br/><i>git.rs</i>"]:::internal
        end

        subgraph filesMod["Files Module"]
            FileTree["File Tree<br/><i>tree.rs</i>"]:::internal
            ReadWrite["Read/Write<br/><i>read_write.rs</i>"]:::internal
            GitDiff["Git Diff<br/><i>git.rs</i>"]:::internal
            Search["Search<br/><i>search.rs</i>"]:::internal
        end
    end

    %% ── Shared crates ────────────────────────────────
    SDK["claude-agent-sdk"]:::crate
    Proto["sandbox-proto"]:::crate
    EventsCrate["moatless-events"]:::crate

    %% ── Backend ↔ Sandbox gRPC (bidirectional) ───────
    Backend -- "ExecuteAgent, StopAgent,<br/>GetAgentStatus, NotifyMessages,<br/>File ops" --> GrpcServer
    GrpcClient -- "PublishEvent,<br/>GetPendingMessages,<br/>AcknowledgeMessages,<br/>GetCredentials" --> Backend

    %% ── K8s probes ───────────────────────────────────
    K8s -- "GET /health" --> HttpServer

    %% ── Internal wiring ──────────────────────────────
    GrpcServer --> Executor
    GrpcServer --> Consumer
    GrpcServer --> filesMod
    Events --> GrpcClient
    Executor --> Options
    Options --> PermHandler
    Executor --> Events
    Consumer --> Executor
    Consumer -.-> |"oneshot channel<br/>(prompt resolution)"| PermHandler
    InitOrch --> Credentials
    InitOrch --> GitInit
    InitOrch --> Consumer
    Credentials --> GrpcClient

    %% ── Agent → Claude CLI ───────────────────────────
    Executor -- "NDJSON stdin/stdout" --> SDK
    SDK -- "spawn + stream" --> ClaudeCLI

    %% ── Init → Git ──────────────────────────────────
    GitInit -- "git clone" --> GitRepo

    %% ── Files → Filesystem ──────────────────────────
    filesMod -- "read/write/tree/diff" --> Filesystem

    %% ── Shared crate usage ───────────────────────────
    GrpcServer -.-> Proto
    GrpcClient -.-> Proto
    Events -.-> EventsCrate
    Executor -.-> SDK
    Consumer -.-> Proto
```

---

## 3. Agent Execution Sequence

Runtime lifecycle of an agent execution, including the interactive prompt (AskUserQuestion) flow.

```mermaid
sequenceDiagram
    participant B as Backend
    participant GS as gRPC Server
    participant AE as Agent Executor
    participant AS as AgentState
    participant EP as Event Publisher
    participant SDK as claude-agent-sdk
    participant CLI as Claude CLI
    participant PH as Permission Handler
    participant MC as Message Consumer
    participant BC as Backend Client

    Note over B,BC: Normal Execution Flow

    B->>GS: ExecuteAgent(content, model, system_prompt)
    GS->>GS: Check agent status (must be Idle)
    GS->>AE: spawn_agent_execution(state, request)
    activate AE

    AE->>AS: set_running(cancel_token)
    AE->>EP: publish(agent.started)
    EP->>BC: PublishEvent (fire-and-forget)

    AE->>AE: fetch_and_apply_credentials()
    AE->>AE: build_options(config, request, handler)
    AE->>SDK: query(prompt, options)
    activate SDK
    SDK->>CLI: spawn subprocess (NDJSON)
    activate CLI

    CLI-->>SDK: System(session_id, model)
    SDK-->>AE: SdkMessage::System
    AE->>AS: set_session_id(id)
    AE->>EP: publish(agent.session_init)

    loop Message Stream
        CLI-->>SDK: Assistant / User messages
        SDK-->>AE: SdkMessage::Assistant / User
        AE->>EP: publish(agent.message)
    end

    Note over B,BC: Interactive Prompt Flow (AskUserQuestion)

    CLI->>PH: can_use_tool("AskUserQuestion", input)
    activate PH
    PH->>PH: Create oneshot channel (tx, rx)
    PH->>AS: set_pending_prompt(PendingPrompt{tx})
    Note over PH: Blocks on rx.await

    B->>GS: NotifyMessages
    GS->>MC: process_pending_messages()
    activate MC
    MC->>BC: get_pending_messages(task_id)
    BC-->>MC: messages[]
    MC->>AS: take_pending_prompt()
    MC->>MC: Extract text, build PromptResponse
    MC-->>PH: send(AskUserAnswer) via oneshot tx
    deactivate MC

    PH-->>CLI: Allow{updated_input}
    deactivate PH
    Note over CLI: Resumes execution

    Note over B,BC: Completion

    CLI-->>SDK: Result(success/error)
    deactivate CLI
    SDK-->>AE: SdkMessage::Result
    deactivate SDK

    AE->>AS: set_session_id(id)
    AE->>EP: publish(agent.completed)
    AE->>AS: set_idle()

    AE->>AE: Spawn deferred: process_pending_messages()
    deactivate AE
```

---

## Key

| Symbol | Meaning |
|--------|---------|
| Solid arrow (`-->`) | Direct function call or data flow |
| Dashed arrow (`-.->`) | Crate dependency / import |
| `oneshot channel` | `tokio::sync::oneshot` — blocks sender until receiver responds |
| `fire-and-forget` | Errors logged but never propagated |
| `NDJSON` | Newline-delimited JSON over stdin/stdout to Claude CLI subprocess |
