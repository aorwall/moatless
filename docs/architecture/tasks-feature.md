# Tasks Feature Architecture

## Status: Complete

**Last Updated**: 2025-10-27

This document describes the architecture of the tasks feature, which owns task lifecycle, agent orchestration, messaging, and command execution. The feature follows a feature-based architecture pattern with focused services and storage adapters.

## Overview

The tasks domain (see `docs/backend-feature-domains.md`) owns task lifecycle, agent orchestration, messaging, and command execution. The feature is organized as a dedicated `features/tasks` module with feature-scoped storage adapters (`docs/patterns/feature-storage-adapters.md`) and a set of focused application services, following the patterns described in `docs/patterns/backend-feature-architecture.md`.

## Architecture Goals

### Design Principles
- **Feature encapsulation**: The `features/tasks` module exposes a clear service API to routes and other features
- **Single responsibility**: Focused services handle specific concerns (lifecycle, execution, event projection, messaging) with explicit dependencies
- **Storage abstraction**: Task-specific storage adapters enable infrastructure provider flexibility
- **Type safety**: Strongly typed boundaries between tasks, iterations, and messages
- **Event-driven coordination**: Reactive event handlers update task state based on agent events

### Scope Boundaries
- **In scope**: Task lifecycle management, agent orchestration, message persistence, command execution
- **Out of scope**: Agent execution implementation details (delegated to AgentService), workspace management (delegated to WorkspaceCoordinator), repository management (delegated to RepositoryService)

## Domain Scope

| Capability | Description |
| --- | --- |
| Task lifecycle | Create/list/get/update/delete tasks, capture metadata, expose derived status, enforce validation. |
| Iteration management | Start iterations, persist git metadata & screenshots, hydrate context tokens, detect errors. |
| Agent orchestration | Provision workspaces, kick off/resume Claude sessions, pipe events to SSE/event service. |
| Messaging & transcripts | Persist user/assistant/tool messages, attachments, contexts, expose UI-friendly transcript. |
| Command context | Surface available commands per task, integrate git status and workspace state. |
| Event projection | Subscribe to agent/task iteration events and reconcile task state in storage. |

## High-Level Architecture

```mermaid
graph TD
  subgraph API
    Routes[REST /tasks, /tasks/:id, /tasks/:id/messages]
  end

  subgraph Tasks Feature
    Service[TaskService]
    Execution[TaskExecutionCoordinator]
    Iterations[TaskIterationService]
    Projector[TaskEventProjector]
    Transcript[MessageService]
    Commands[TaskCommandGateway]
    Storage[(TaskMetadataStorage)]
    TranscriptStore[(MessageStorage)]
  end

  subgraph Other Features
    Workspace[WorkspaceCoordinator/FileGateway]
    Agents[AgentService/AgentController]
    Preview[PreviewManager]
    Repos[RepositoryService]
    Screenshots[ScreenshotOrchestrationService]
    Events[EventService]
    Uploads[UploadService]
  end

  Routes --> Service
  Service --> Storage
  Service --> Execution
  Service --> Transcript
  Service --> Commands
  Service --> Repos
  Iterations --> Storage
  Transcript --> TranscriptStore
  Execution --> Storage
  Execution --> Workspace
  Execution --> Agents
  Execution --> Preview
  Execution --> Screenshots
  Commands --> Storage
  Commands --> Workspace
  Projector --> Storage
  Projector --> Transcript
  Events --> Projector
  Transcript --> Uploads
```

## Directory Layout

```
apps/backend/src/features/tasks/
  services/
    message.service.ts
    task-lifecycle.service.ts
    task-iteration.service.ts
    task-execution-coordinator.service.ts
    task-event-projector.service.ts
    task-command-gateway.service.ts
    task.service.ts
    __tests__/
      message.service.unit.test.ts
      message.service.integration.test.ts
      task-lifecycle.service.test.ts
      task-iteration.service.test.ts
      task-execution-coordinator.service.test.ts
      task-event-projector.service.test.ts
      task-command-gateway.service.test.ts
  storage/
    task-metadata.storage.ts
    message.storage.ts
    __tests__/
      task-metadata.storage.test.ts
      task-message.storage.test.ts
    index.ts
  utils/
    task-status.utils.ts
  api/
    tasks.routes.ts
    worker.routes.ts
  index.ts
```

**Design Notes**:
- Uses `services/` directory (not `application/`) per `docs/patterns/backend-feature-architecture.md`
- Iterations stored within task metadata - no separate iteration storage needed
- Event subscription handled in `TaskEventProjector` service - no separate subscribers directory
- All services bound in `container/container.ts` as singletons using InversifyJS

## Application Services & Responsibilities

| Service | Responsibilities | Key Dependencies |
| --- | --- | --- |
| `TaskService` | Public facade for all task operations. Handles CRUD (get/list/update with status derivation and enrichment). Orchestrates complex workflows (task creation with workspace setup, deletion with cleanup). Delegates execution to specialized services. | `TaskMetadataStorage`, `TaskExecutionCoordinator`, `MessageService`, `TaskCommandGateway`, `WorkspaceCoordinator`, `RepositoryService`, `EventService`, `AgentController`, `PreviewManager` |
| `TaskIterationService` | Iteration lifecycle management. Create iterations with validation. Apply results/errors. Capture git metadata (status, changes, commit hash). Create checkpoints and push changes. Coordinate screenshot captures. Emit iteration events. | `TaskMetadataStorage`, `FileGateway`, `ScreenshotOrchestrationService`, `EventService`, `RepositoryService` |
| `TaskExecutionCoordinator` | Orchestrate agent execution and workspace management. Send messages/commands to tasks. Start tasks and wait for first event. Provision workspaces and validate readiness. Execute agents in background with error handling. Resume logic for continuing iterations. Handles workspace setup during task creation. | `TaskMetadataStorage`, `TaskIterationService`, `MessageService`, `WorkspaceCoordinator`, `FileGateway`, `AgentController`, `EventService`, `RepositoryService`, `CommandService`, `UploadService`, `FigmaService` |
| `TaskEventProjector` | Reactive event handler for task state updates. Subscribe to `EventService` and reconcile task state based on agent events (`agent.completed`, `agent.error`, `agent.message`). Update context tokens and agent session IDs. Trigger post-processing workflows. | `EventService`, `TaskMetadataStorage`, `TaskIterationService` |
| `TaskCommandGateway` | Filter available commands based on task and workspace state. Build command context from git status and iteration history. Apply command condition filtering using `filterAvailableCommands()` utility. Reads tasks directly from storage (no enrichment needed). | `CommandService`, `FileGateway`, `TaskMetadataStorage` |
| `MessageService` | Persist user/assistant/tool messages to JSONL. Fetch and transform transcripts for UI consumption. Stitch contexts and file attachments for display. Handle message deduplication. | `MessageStorage`, `UploadService`, `TaskMetadataStorage` |

## Storage & Persistence Strategy

Each logical collection is backed by a dedicated adapter per `docs/patterns/feature-storage-adapters.md`:

- **`TaskMetadataStorage`** → `tasks/{taskId}.json` (full `Task` aggregate including iterations)
  - Uses `InfrastructureStorageSymbol` for storage abstraction
  - Methods: `save()`, `get()`, `delete()`, `exists()`, `listTaskKeys()`

- **`MessageStorage`** → `messages/{taskId}.jsonl` (append-only log for transcripts)
  - Uses `StorageSymbol` (`StorageBackend`) for JSONL operations
  - Methods: `read()`, `append()`, `delete()`, `exists()`
  - **Note**: JSONL operations (readJsonl/appendJsonl) not available in minimal `InfrastructureStorageSymbol` interface

**Implementation Example**:
```ts
@injectable()
export class TaskMetadataStorage {
  constructor(@inject(InfrastructureStorageSymbol) private readonly storage: Storage) {}

  async save(task: Task): Promise<void> {
    await this.storage.save(this.getKey(task.taskId), task)
  }

  async get(taskId: string): Promise<Task | null> {
    return this.storage.get<Task>(this.getKey(taskId))
  }

  private getKey(taskId: string): string {
    return `tasks/${taskId}`
  }
}
```

**Container Bindings**:
```ts
container.bind(TaskMetadataStorage).toSelf().inSingletonScope()
container.bind(MessageStorage).toSelf().inSingletonScope()
```

The adapters keep feature services free from infrastructure knowledge. Tests use mocked `Storage` implementations following the pattern doc.

## Message & Event Flow

### Task Transcript Pipeline
1. `TaskExecutionCoordinator` or routes call `MessageService.saveUserMessage` before starting/resuming the agent.
2. `TaskEventProjector` forwards every `agent.message` event to `MessageService.saveAgentMessage`, deduping initial user prompts just like today.
3. `MessageService.getTaskMessages` hydrates attachment metadata via `UploadService` and returns UI-friendly `Message[]` values.
4. Downstream consumers (UI, worker mode) only depend on the transcript service; no other module reads JSONL files directly.

### Agent Event Projection
1. `TaskEventProjector` subscribes to `EventService.subscribeAll` during feature bootstrap.
2. For each relevant event it loads the task aggregate once via `TaskMetadataStorage`, delegates business logic to `TaskIterationService`, persists changes, and emits follow-up task / iteration events when state changes.
3. Context token updates (`agent.message`) and agent session IDs are applied opportunistically without blocking other event handlers.

## API Surface & External Contracts

- `POST /tasks` → `TaskService.createTask` (supports template/vibe creation, immediate start flag, contexts, file attachments).
- `POST /tasks/vibe` → `TaskService.createVibeTask` (repository bootstrap + task creation).
- `POST /tasks/:id/start` / `POST /tasks/:id/messages` → `TaskExecutionCoordinator.startTask` / `sendMessage`.
- `GET /tasks/:id` / `GET /tasks` → `TaskService.getTask` / `listTasks` with derived status and repository metadata.
- `GET /tasks/:id/messages` → `MessageService.getTaskMessages`.
- `POST /tasks/:id/close`, `DELETE /tasks/:id` → Task lifecycle operations that also call workspace/preview cleanup.
- `GET /tasks/:id/commands` → `TaskCommandGateway.getAvailableCommands` leveraging filtered command metadata.

## Class Diagram

```mermaid
classDiagram
    class TaskService {
      -TaskMetadataStorage taskStorage
      -RepositoryService repositoryService
      -TaskIterationService iterationService
      -TaskExecutionCoordinator execution
      -MessageService transcript
      -TaskCommandGateway commands
      -WorkspaceCoordinator workspaceCoordinator
      -PreviewManager previewManager
      -AgentController agentController
      -EventService eventService
      +createTask(request: CreateTaskRequest): Promise<Task>
      +createVibeTask(request: CreateVibeTaskRequest): Promise<string>
      +startTask(taskId: string, fileIds?: string[], contexts?: MessageContext[]): Promise<void>
      +sendMessage(taskId: string, input: SendMessageInput): Promise<void>
      +getTask(taskId: string): Promise<Task>
      +listTasks(limit?: number, offset?: number, status?: string): Promise<TaskPage>
      +updateTaskMetadata(taskId: string, updates: TaskMetadataUpdate): Promise<void>
      +getTaskMessages(taskId: string): Promise<Message[]>
      +closeTask(taskId: string): Promise<void>
      +deleteTask(taskId: string): Promise<void>
      +getAvailableCommands(taskId: string): Promise<Command[]>
    }

    class TaskIterationService {
      -TaskMetadataStorage taskStorage
      -FileGateway fileGateway
      -ScreenshotOrchestrationService screenshotService
      +createIteration(task: Task, input: IterationInput): TaskIteration
      +applyResult(iteration: TaskIteration, payload: SDKResultMessage): void
      +applyError(iteration: TaskIteration, payload: AgentErrorPayload): void
      +captureGitData(taskId: string, iteration: TaskIteration): Promise<void>
      +createCheckpoint(task: Task, iteration: TaskIteration): Promise<void>
      +pushChanges(task: Task, iteration: TaskIteration): Promise<void>
      +emitIterationEvents(task: Task, iteration: TaskIteration, previousStatus: TaskStatus): Promise<void>
    }

    class TaskExecutionCoordinator {
      -TaskMetadataStorage taskStorage
      -WorkspaceCoordinator workspaceCoordinator
      -TaskIterationService iterationService
      -MessageService transcriptService
      -AgentService agentService
      -AgentController agentController
      -PreviewManager previewManager
      -EventService eventService
      -RepositoryService repositoryService
      -CommandService commandService
      +startTask(taskId: string, options: StartTaskOptions): Promise<void>
      +sendMessage(taskId: string, input: SendMessageInput): Promise<void>
      +sendCommand(taskId: string, commandName: string): Promise<void>
      +resumeTask(taskId: string, resumeSessionId: string, input: SendMessageInput): Promise<void>
      +createAndSetupWorkspace(task: Task, repositoryId: string): Promise<void>
      +handleTaskStartOrSetup(task: Task, params: CreateTaskRequest): Promise<void>
      +setupWorkspaceAndWaitForAgentStart(taskId: string, start: boolean, message?: string): Promise<void>
      +waitForFirstEvent(taskId: string): Promise<void>
      +handleWorkspaceSetupFailure(taskId: string, error: unknown): Promise<void>
    }

    class TaskEventProjector {
      -EventService eventService
      -TaskMetadataStorage taskStorage
      -TaskIterationService iterationService
      -MessageService transcriptService
      +subscribe(): void
      +handleEvent(event: Event): Promise<void>
      +handleResultEvent(task: Task, iteration: TaskIteration, event: Event): Promise<void>
      +handleErrorEvent(task: Task, iteration: TaskIteration, event: Event): Promise<void>
      +handleMessageEvent(task: Task, iteration: TaskIteration, event: Event): Promise<void>
    }

    class MessageService {
      -MessageStorage transcriptStorage
      -UploadService uploadService
      +getTaskMessages(taskId: string): Promise<Message[]>
      +saveUserMessage(taskId: string, message: string, options: TranscriptOptions): Promise<void>
      +saveAgentMessage(taskId: string, event: Event): Promise<void>
      +transformAgentMessagesToMessages(messages: AgentMessage[]): Message[]
    }

    class TaskCommandGateway {
      -CommandService commandService
      -FileGateway fileGateway
      -TaskMetadataStorage taskStorage
      +getAvailableCommands(taskId: string): Promise<Command[]>
      +buildCommandContext(task: Task): Promise<CommandContext>
    }

    class TaskMetadataStorage {
      -Storage infrastructureStorage
      +get(taskId: string): Promise<Task | null>
      +save(task: Task): Promise<void>
      +delete(taskId: string): Promise<void>
      +listTaskKeys(): Promise<string[]>
    }

    class MessageStorage {
      -Storage infrastructureStorage
      +read(taskId: string): Promise<AgentMessage[]>
      +append(taskId: string, message: AgentMessage): Promise<void>
      +delete(taskId: string): Promise<void>
    }

    TaskService --> TaskMetadataStorage
    TaskService --> RepositoryService
    TaskService --> TaskIterationService
    TaskService --> TaskExecutionCoordinator
    TaskService --> MessageService
    TaskService --> TaskCommandGateway
    TaskIterationService --> TaskMetadataStorage
    TaskExecutionCoordinator --> TaskMetadataStorage
    TaskExecutionCoordinator --> TaskIterationService
    TaskExecutionCoordinator --> MessageService
    TaskExecutionCoordinator --> AgentService
    TaskEventProjector --> TaskIterationService
    TaskEventProjector --> MessageService
    MessageService --> MessageStorage
    TaskCommandGateway --> TaskMetadataStorage
```

## Sequence Diagrams

### Task Creation With Immediate Start

```mermaid
sequenceDiagram
    participant Client
    participant TasksRoute
    participant TaskService
    participant Storage as TaskMetadataStorage
    participant Workspace as WorkspaceCoordinator
    participant Transcript as MessageService
    participant Execution as TaskExecutionCoordinator
    participant Agent as AgentService
    participant Events as EventService

    Client->>TasksRoute: POST /tasks (start=true, message)
    TasksRoute->>TaskService: createTask(request)
    TaskService->>Storage: save(task)
    TaskService->>Execution: createAndSetupWorkspace(task, repositoryId)
    Execution->>Workspace: ensureWorkspace(taskId, repositoryId)
    Execution->>Storage: save(task with workspacePath)
    TaskService->>Execution: handleTaskStartOrSetup(task, request)
    Execution->>Transcript: saveUserMessage(taskId, message, contexts/files)
    Execution->>Workspace: ensureWorkspace(taskId)
    Execution->>Agent: executeAgent(agentRequest)
    Agent-->>Events: stream agent events
    Events-->>Client: SSE /task events
```

### Agent Event Projection

```mermaid
sequenceDiagram
    participant Events as EventService
    participant Projector as TaskEventProjector
    participant Storage as TaskMetadataStorage
    participant Iterations as TaskIterationService
    participant Transcript as MessageService
    participant EventsOut as EventService

    Events-->>Projector: agent.completed / agent.error / agent.message
    Projector->>Storage: get(taskId)
    Projector->>Iterations: update iteration (result/error/message)
    Iterations-->>Projector: updated iteration data
    Projector->>Transcript: saveAgentMessage(taskId, event)
    Projector->>Storage: save(updatedTask)
    Projector->>EventsOut: emit task/taskIteration events (status changes)
```


The routes provide backward-compatible endpoints while exposing the feature's public API. Other domains (worker mode, usage tracking) should import `TaskService` rather than accessing internal services directly.
