# Workspace Architecture

## Table of Contents
1. [Overview](#overview)
2. [Architectural Principles](#architectural-principles)
3. [Core Architecture](#core-architecture)
4. [Isolation Architecture](#isolation-architecture)
5. [Lifecycle Management](#lifecycle-management)
6. [Security Architecture](#security-architecture)
7. [Integration Architecture](#integration-architecture)
8. [Performance & Scalability](#performance--scalability)
9. [Monitoring & Observability](#monitoring--observability)
10. [Best Practices](#best-practices)

## Overview

### Purpose
The Workspace system creates **isolated execution environments** for AI agents, enabling safe and concurrent task execution while maintaining complete separation between tasks and protecting the main repository.

### Key Capabilities
```mermaid
graph TB
    subgraph "Workspace Capabilities"
        ISO[Isolation & Security]
        GIT[Git Worktree Management]
        EXE[Command Execution]
        FS[File System Operations]
        LC[Lifecycle Management]
        MON[Resource Monitoring]
    end
    
    ISO --> |"Process isolation"| EXE
    ISO --> |"Path sandboxing"| FS
    GIT --> |"Branch isolation"| LC
    LC --> |"Setup/Teardown"| MON
```

### Strategic Value
- **Complete Task Isolation**: Every task operates in its own environment
- **Concurrent Execution**: Multiple agents work simultaneously without conflicts
- **Version Control Safety**: Git worktrees prevent code conflicts
- **Resource Management**: Controlled allocation and cleanup
- **Security Boundaries**: Sandboxed execution prevents system access

### Workspace Capabilities

The workspace service provides capability checking to ensure operations are only performed when the workspace is in an appropriate state:

1. **File Operations** (`apps/backend/services/workspace.service.ts`)
   - `canListFiles()`: Available when workspace exists (any state except NOT_SETUP)
   - `canReadWriteFiles()`: Available during setup, idle, or running states
   - `canSearchFiles()`: Available when workspace is initialized

2. **Command Execution**
   - `canExecuteCommands()`: Only when workspace is in IDLE state
   - Prevents command execution during setup or when agent is running
   - Ensures exclusive access to command execution

3. **Git Operations**
   - `canAccessGit()`: Available when workspace exists
   - Includes diff generation, branch info, commit status
   - Works even during active development

4. **Server Management** (Preview Environments)
   - `canManageServer()`: Available in IDLE or RUNNING states
   - Start/stop/restart development servers
   - Port forwarding and log access

5. **State-Based Availability**
   ```
   NOT_SETUP:     No operations available
   SETTING_UP:    File operations only
   IDLE:          All operations available
   RUNNING:       File/git operations, no commands
   ERROR:         Limited to cleanup operations
   ```

## Architectural Principles

### Core Design Principles

```mermaid
graph LR
    subgraph "Design Principles"
        ISO[Task Isolation]
        SEC[Security First]
        EFF[Resource Efficiency]
        CON[Concurrency Support]
        TRA[Traceability]
    end
    
    ISO --> |"One workspace per task"| SEC
    SEC --> |"Sandboxed execution"| EFF
    EFF --> |"Git worktrees"| CON
    CON --> |"Parallel execution"| TRA
```

1. **Complete Isolation**: Each task operates in a completely separate environment
2. **Security by Design**: All operations are sandboxed and validated
3. **Resource Efficiency**: Shared git objects, lazy initialization
4. **Concurrent Safety**: Multiple agents can work simultaneously
5. **Full Traceability**: All operations are logged and tracked

### Architectural Constraints

- **No Shared State**: Workspaces never share data or resources
- **Immutable Infrastructure**: Workspaces are disposable and recreatable
- **Fail-Safe Defaults**: Security restrictions enabled by default
- **Graceful Degradation**: Failures isolated to individual workspaces

## Core Architecture

### System Architecture

```mermaid
graph TB
    subgraph "Client Layer"
        UI[UI Components]
        API[API Client]
    end
    
    subgraph "Service Layer"
        WS[Workspace Service]
        TS[Task Service]
        AS[Agent Service]
    end
    
    subgraph "Workspace Layer"
        WM[Workspace Manager]
        LW[Local Workspace]
        DW[Docker Workspace]
        VW[Virtual Workspace]
    end
    
    subgraph "Infrastructure Layer"
        GIT[Git Worktrees]
        FS[File System]
        DOC[Docker Engine]
        SEC[Security Layer]
    end
    
    UI --> API
    API --> WS
    TS --> WS
    AS --> WS
    
    WS --> WM
    WM --> LW
    WM --> DW
    WM --> VW
    
    LW --> GIT
    LW --> FS
    DW --> DOC
    VW --> SEC
    
    style WS fill:#f9f,stroke:#333,stroke-width:4px
    style WM fill:#bbf,stroke:#333,stroke-width:2px
```

**Implementation Details:**

The workspace system architecture is organized into distinct layers:

1. **Service Layer** - Core business logic
   - `apps/backend/services/workspace.service.ts` - Main workspace service orchestrating all workspace operations
   - `apps/backend/services/task.service.ts` - Task service that requests workspace creation
   - `apps/backend/services/agents/` - Agent implementations that execute within workspaces

2. **Workspace Layer** - Workspace implementations
   - `apps/backend/workspaces/base.ts` - Abstract base class defining workspace interface
   - `apps/backend/workspaces/local.ts` - Local filesystem workspace using git worktrees
   - `apps/backend/workspaces/docker.ts` - Docker container-based workspace implementation
   - Virtual workspace (planned for future implementation)

3. **Infrastructure Layer** - Low-level utilities
   - `apps/backend/utils/git-utils.ts` - Git operations including worktree management
   - `apps/backend/utils/file-search.ts` - File system search utilities
   - Docker integration through shell commands
   - Path validation and security utilities

### Component Architecture

```mermaid
graph LR
    subgraph "Workspace Service"
        direction TB
        API[API Layer]
        ORCH[Orchestration]
        VAL[Validation]
        LIFE[Lifecycle Manager]
    end
    
    subgraph "Workspace Types"
        direction TB
        LOCAL[Local Workspace]
        DOCKER[Docker Workspace]
        REMOTE[Remote Workspace]
    end
    
    subgraph "Core Services"
        direction TB
        GIT[Git Service]
        FS[File Service]
        CMD[Command Service]
        SEC[Security Service]
    end
    
    API --> ORCH
    ORCH --> VAL
    ORCH --> LIFE
    
    LIFE --> LOCAL
    LIFE --> DOCKER
    LIFE --> REMOTE
    
    LOCAL --> GIT
    LOCAL --> FS
    LOCAL --> CMD
    DOCKER --> SEC
```

### Workspace Hierarchy

```mermaid
classDiagram
    class BaseWorkspace {
        <<abstract>>
        +taskId: string
        +status: WorkspaceStatus
        +setup(): Promise~void~
        +teardown(): Promise~void~
        +executeCommand(): Promise~CommandResult~
        +listFiles(): Promise~FileInfo[]~
    }
    
    class LocalWorkspace {
        -workspacePath: string
        -branchName: string
        +createGitWorktree()
        +runSetupCommands()
    }
    
    class DockerWorkspace {
        -containerId: string
        -containerName: string
        +createContainer()
        +copyRepositoryFiles()
    }
    
    class VirtualWorkspace {
        -memoryFS: Map
        +simulateCommand()
        +mockFileSystem()
    }
    
    BaseWorkspace <|-- LocalWorkspace
    BaseWorkspace <|-- DockerWorkspace
    BaseWorkspace <|-- VirtualWorkspace
```

**Workspace Class Implementation Details:**

1. **BaseWorkspace Abstract Class** (`apps/backend/workspaces/base.ts`)
   - Defines common interface for all workspace types
   - Properties:
     - `taskId`: Unique identifier linking to task
     - `repositoryConfig`: Configuration from repository service
     - `eventService`: For emitting workspace events
   - Abstract methods that subclasses must implement:
     - `setup()`: Initialize workspace environment
     - `teardown()`: Clean up resources
     - `runCommand()`: Execute shell commands
     - `listFiles()`: Directory traversal
     - `readFile()`: File content retrieval
     - `writeFile()`: File modification
     - `searchFiles()`: File search functionality
     - `getGitDiff()`: Version control changes

2. **LocalWorkspace Implementation** (`apps/backend/workspaces/local.ts`)
   - Uses git worktrees for isolation
   - Key methods:
     - `createGitWorktree()`: Sets up isolated git working directory
     - `runSetupCommands()`: Executes npm install, pip install, etc.
     - `validatePath()`: Ensures operations stay within workspace
     - `executeCommand()`: Runs commands with timeout and resource limits
   - State management:
     - Tracks setup progress
     - Monitors running processes
     - Manages background tasks

3. **DockerWorkspace Implementation** (`apps/backend/workspaces/docker.ts`)
   - Container-based isolation
   - Key methods:
     - `createContainer()`: Docker container initialization
     - `copyRepositoryFiles()`: Syncs code to container
     - `executeInContainer()`: Runs commands inside Docker
     - `getContainerLogs()`: Retrieves execution logs
   - Container lifecycle:
     - Creates named containers per task
     - Manages volume mounts
     - Handles container cleanup

4. **VirtualWorkspace (Planned)**
   - In-memory workspace for testing
   - Would provide:
     - Memory-based file system
     - Command simulation
     - Fast test execution
     - No disk I/O

## Isolation Architecture

### Git Worktree Isolation

```mermaid
graph TB
    subgraph "Main Repository"
        MAIN[Main Branch]
        OBJ[Git Objects]
        REF[References]
    end
    
    subgraph "Workspace A"
        WA[Worktree A]
        BA[Branch: workspace-123]
        FA[Files & Changes]
    end
    
    subgraph "Workspace B"
        WB[Worktree B]
        BB[Branch: workspace-456]
        FB[Files & Changes]
    end
    
    subgraph "Workspace C"
        WC[Worktree C]
        BC[Branch: workspace-789]
        FC[Files & Changes]
    end
    
    OBJ --> |"Shared objects"| WA
    OBJ --> |"Shared objects"| WB
    OBJ --> |"Shared objects"| WC
    
    WA --> BA
    WB --> BB
    WC --> BC
    
    style OBJ fill:#f96,stroke:#333,stroke-width:2px
```

**Git Worktree Implementation Details:**

Git worktrees provide workspace isolation through separate working directories while sharing the same git repository objects. Here's how it works:

1. **Worktree Creation** (`apps/backend/utils/git-utils.ts:createGitWorktree()`)
   - Each task gets its own worktree with a dedicated branch
   - Branch naming convention: `workspace-<task-id>`
   - Worktree path: `<repo-parent>/<repo-name>-workspace-<task-id>`
   - Creation command: `git worktree add -b "workspace-<task-id>" "<path>"`

2. **Object Sharing Benefits**
   - Git objects (.git/objects) are shared across all worktrees
   - Reduces disk usage by ~90% compared to full clones
   - Faster creation time (seconds vs minutes for large repos)
   - Automatic garbage collection manages shared objects

3. **Branch Isolation** (`apps/backend/workspaces/local.ts`)
   - Each workspace operates on its own branch
   - Changes are isolated from main repository and other workspaces
   - Prevents conflicts between concurrent AI agents
   - Branches can be merged back to main when task completes

4. **Conflict Prevention**
   - Git prevents same branch from being checked out in multiple worktrees
   - System automatically creates new branch if conflict detected
   - Branch tracking ensures proper upstream relationships

**Key Benefits:**
- **Shared Git Objects**: Efficient disk usage through object sharing
- **Independent Branches**: Each workspace has its own branch
- **Parallel Development**: Multiple agents work without conflicts
- **Clean Separation**: Changes isolated until explicitly merged

### Docker Container Isolation

```mermaid
graph TB
    subgraph "Host System"
        HS[Host OS]
        HF[Host File System]
        NET[Network Stack]
    end
    
    subgraph "Container A"
        CA[Isolated Process]
        FA[Container FS]
        NA[Network Namespace]
        RA[Resource Limits]
    end
    
    subgraph "Container B"
        CB[Isolated Process]
        FB[Container FS]
        NB[Network Namespace]
        RB[Resource Limits]
    end
    
    HS --> |"Process isolation"| CA
    HS --> |"Process isolation"| CB
    HF --> |"Mount namespace"| FA
    HF --> |"Mount namespace"| FB
    NET --> |"Network isolation"| NA
    NET --> |"Network isolation"| NB
    
    style CA fill:#9f9,stroke:#333,stroke-width:2px
    style CB fill:#9f9,stroke:#333,stroke-width:2px
```

**Docker Container Implementation Details:**

Docker provides complete process and resource isolation for workspace execution:

1. **Container Creation** (`apps/backend/workspaces/docker.ts:setup()`)
   - Container naming: `moatless-task-<task-id>`
   - Base image configurable per repository (default: `node:18-alpine`)
   - Working directory: `/workspace`
   - Container creation: `docker create --name "<name>" -w "/workspace" "<image>" sleep infinity`

2. **File System Isolation**
   - Repository files copied to container: `docker cp "<repo>/." "<container>:/workspace/"`
   - Complete filesystem isolation from host and other containers
   - Volume mounts supported for persistent data
   - Changes confined to container filesystem

3. **Process Isolation Features**
   - **Namespace Isolation**: PID, network, mount, UTS, IPC namespaces
   - **Resource Limits**: Configurable CPU and memory constraints
   - **Security**: Capabilities dropped, seccomp profiles applied
   - **User Isolation**: Runs as non-root user when configured

4. **Network Isolation**
   - Each container gets its own network namespace
   - Port mapping required for external access
   - Inter-container communication restricted by default
   - Network policies enforceable via Docker networks

5. **Resource Management** (`apps/backend/workspaces/docker.ts`)
   - CPU limits prevent resource hogging
   - Memory limits prevent OOM conditions
   - I/O throttling available for disk operations
   - Automatic cleanup on container stop

**Container Benefits:**
- **Complete Process Isolation**: No cross-task interference
- **Resource Limits**: CPU, memory, and I/O constraints
- **Network Isolation**: Separate network namespaces
- **File System Isolation**: Independent file systems

### Process Isolation Strategy

```mermaid
stateDiagram-v2
    [*] --> Created: Task assigned
    Created --> Initializing: Setup workspace
    Initializing --> Ready: Setup complete
    Initializing --> Failed: Setup error
    Ready --> Active: Agent starts
    Active --> Ready: Agent completes
    Active --> Failed: Execution error
    Ready --> Cleaning: Cleanup requested
    Failed --> Cleaning: Force cleanup
    Cleaning --> [*]: Resources freed
    
    note right of Active
        - Sandboxed execution
        - Resource monitoring
        - Path validation
    end note
    
    note right of Cleaning
        - Git worktree removal
        - Container teardown
        - Resource cleanup
    end note
```

**Process Isolation Implementation:**

The workspace system implements a comprehensive state machine for process lifecycle management:

1. **State Transitions** (`packages/shared/types/workspace.ts:WorkspaceStatus`)
   - `NOT_SETUP`: Initial state before workspace creation
   - `SETTING_UP`: Workspace being created and configured
   - `IDLE`: Ready for agent execution
   - `RUNNING`: Agent actively executing
   - `ERROR`: Setup or execution failure

2. **Workspace Initialization** (`apps/backend/workspaces/local.ts:setup()`)
   - Validates workspace path exists or creates it
   - Sets up git worktree for version control isolation
   - Runs background setup commands (npm install, etc.)
   - Transitions to IDLE state when ready

3. **Agent Execution Isolation**
   - **Process Spawning**: Uses Node.js `child_process.exec` with constraints
   - **Working Directory**: Always set to workspace root
   - **Environment Variables**: Filtered and workspace-specific
   - **Signal Handling**: Proper SIGTERM/SIGKILL escalation

4. **Resource Monitoring** (`apps/backend/workspaces/local.ts`)
   - Tracks active processes per workspace
   - Monitors command execution time
   - Enforces timeout limits (default 2 min, max 10 min)
   - Prevents resource exhaustion

5. **Cleanup Process** (`apps/backend/workspaces/local.ts:teardown()`)
   - Terminates all running processes
   - Removes git worktree and branch
   - Cleans up temporary files
   - Updates workspace state to reflect cleanup

## Lifecycle Management

### Workspace Creation Flow

```mermaid
sequenceDiagram
    participant Client
    participant TaskService
    participant WorkspaceService
    participant GitWorktree
    participant FileSystem
    
    Client->>TaskService: Create task
    TaskService->>WorkspaceService: Request workspace
    
    alt Git Worktree Strategy
        WorkspaceService->>GitWorktree: Create worktree
        GitWorktree->>GitWorktree: New branch
        GitWorktree->>FileSystem: Create directory
        GitWorktree-->>WorkspaceService: Worktree ready
    else Docker Strategy
        WorkspaceService->>Docker: Create container
        Docker->>Docker: Setup isolation
        Docker-->>WorkspaceService: Container ready
    end
    
    WorkspaceService->>WorkspaceService: Run setup commands
    WorkspaceService-->>TaskService: Workspace ready
    TaskService-->>Client: Task created
```

**Detailed Workspace Creation Process:**

1. **Task Creation** (`apps/backend/services/task.service.ts`)
   - Client initiates task creation through API endpoint
   - Task service validates repository configuration
   - Generates unique task ID for workspace isolation

2. **Workspace Request** (`apps/backend/services/workspace.service.ts:createWorkspace()`)
   - WorkspaceService receives task ID and repository ID
   - Retrieves repository configuration to determine workspace type
   - Decides between Local (git worktree) or Docker workspace based on `repository.codeExecution.type`

3. **Git Worktree Creation** (for Local workspaces)
   - **Path Generation** (`workspace.service.ts:createWorkspaceRepository()`)
     - Constructs workspace path: `<WORKTREE_DIR>/<repo-name>-workspace-<task-id>`
     - Default worktree directory: `../` relative to repository path
     - Can be overridden with `WORKTREE_DIR` environment variable
   
   - **Branch Creation** (`utils/git-utils.ts:createGitWorktree()`)
     - Creates new branch named `workspace-<task-id>`
     - If branch exists and is checked out, creates alternative branch
     - Executes: `git worktree add -b "<branch-name>" "<workspace-path>"`
   
   - **Validation**
     - Verifies repository is a git repository
     - Ensures worktree directory exists and is writable
     - Handles existing worktrees gracefully

4. **Docker Container Creation** (for Docker workspaces)
   - **Container Setup** (`apps/backend/workspaces/docker.ts:setup()`)
     - Creates container with name `moatless-task-<task-id>`
     - Configures working directory as `/workspace`
     - Applies volume mounts and environment variables from config
   
   - **File Synchronization**
     - Copies repository files to container: `docker cp "<repo-path>/." "<container-id>:/workspace/"`
     - Preserves file permissions and structure
   
   - **Container Initialization**
     - Starts container with `sleep infinity` to keep it running
     - Executes setup commands defined in repository config

5. **Setup Commands Execution**
   - **Local Workspace** (`apps/backend/workspaces/local.ts:runSetupCommandsInBackground()`)
     - Runs commands like `npm install`, `pip install`, etc.
     - Executes in background to allow immediate workspace availability
     - Emits events for command progress tracking
   
   - **Docker Workspace**
     - Runs setup commands inside container
     - Each command execution is tracked and logged
     - Failures in setup commands mark workspace as errored

6. **Workspace Registration**
   - Workspace instance stored in `activeWorkspaces` Map
   - Workspace path saved to task metadata for recovery
   - Status transitions from `initializing` to `ready`

### Workspace Teardown Flow

```mermaid
graph TB
    subgraph "Teardown Process"
        CHECK[Check Status]
        SAVE[Save Changes?]
        STOP[Stop Processes]
        CLEAN[Clean Resources]
        REMOVE[Remove Workspace]
    end
    
    subgraph "Git Cleanup"
        GIT1[Remove Worktree]
        GIT2[Delete Branch]
        GIT3[Prune References]
    end
    
    subgraph "Docker Cleanup"
        DOC1[Stop Container]
        DOC2[Remove Container]
        DOC3[Clean Volumes]
    end
    
    CHECK --> SAVE
    SAVE --> |"Has changes"| STOP
    SAVE --> |"No changes"| STOP
    STOP --> CLEAN
    CLEAN --> |"Git workspace"| GIT1
    CLEAN --> |"Docker workspace"| DOC1
    GIT1 --> GIT2
    GIT2 --> GIT3
    DOC1 --> DOC2
    DOC2 --> DOC3
    GIT3 --> REMOVE
    DOC3 --> REMOVE
```

**Detailed Workspace Teardown Process:**

1. **Cleanup Initiation** (`apps/backend/services/workspace.service.ts:cleanupWorkspace()`)
   - Triggered when task completes, fails, or is manually stopped
   - Logs cleanup start time for performance tracking
   - Retrieves workspace from `activeWorkspaces` Map

2. **Status Verification**
   - Checks current workspace status and type
   - Determines if force cleanup is needed (always true in current implementation)
   - Captures workspace path before cleanup for logging

3. **Local Workspace Teardown** (`apps/backend/workspaces/local.ts:teardown()`)
   - **Process Termination**
     - Stops any running agent processes
     - Kills server processes if preview environment is active
     - Ensures all child processes are terminated
   
   - **Git Worktree Removal** (`utils/git-utils.ts:removeGitWorktree()`)
     - Executes: `git worktree remove --force "<workspace-path>"`
     - Force flag ensures removal even with uncommitted changes
     - Handles cases where worktree is already removed
   
   - **Branch Cleanup**
     - Deletes workspace branch: `git branch -D workspace-<task-id>`
     - Prunes worktree references: `git worktree prune`
     - Cleans up any dangling references

4. **Docker Workspace Teardown** (`apps/backend/workspaces/docker.ts:teardown()`)
   - **Container Stopping**
     - Gracefully stops container: `docker stop "<container-id>"`
     - Waits for container to stop with timeout
     - Force kills if graceful stop fails
   
   - **Container Removal**
     - Removes container: `docker rm -f "<container-id>"`
     - Cleans up associated volumes if configured
     - Removes any temporary files created during execution
   
   - **Resource Cleanup**
     - Releases allocated ports
     - Cleans up network configurations
     - Removes any created volumes

5. **Workspace Deregistration**
   - Removes workspace from `activeWorkspaces` Map
   - Updates task metadata to reflect workspace removal
   - Emits workspace deleted event for audit trail

6. **Error Handling During Teardown**
   - Catches and logs all errors without rethrowing
   - Ensures workspace is removed from active list even if teardown fails
   - Provides detailed error logging for debugging
   - Total cleanup time is tracked and logged
