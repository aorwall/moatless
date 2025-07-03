# Workspace Architecture

## Overview

The Moatless Swarm UI workspace architecture provides isolated execution environments for AI agents to safely modify code repositories. The system supports both local git worktree-based isolation and full Docker containerization, with integrated preview environments for testing changes.

## Architecture Components

### 1. Repository Management

Repositories are the foundation of the workspace system. Each repository represents a codebase that tasks can operate on.

```mermaid
graph TB
    subgraph "Repository Configuration"
        RC[RepositoryConfig]
        RC --> LP[Local Path]
        RC --> CE[Code Execution Settings]
        RC --> PC[Preview Config]
        
        CE --> LT[Local Type]
        CE --> DT[Docker Type]
        CE --> SC[Setup Commands]
        CE --> SV[Server Command]
        
        PC --> FW[Framework Detection]
        PC --> SI[Script Injection]
        PC --> PS[Preview Settings]
    end
    
    subgraph "Repository Service"
        RS[Repository Service]
        RS --> CR[Create Repository]
        RS --> LR[List Repositories]
        RS --> FS[File Search]
        RS --> GB[Get Branches]
        RS --> DF[Detect Framework]
    end
    
    RC --> RS
```

#### Repository Configuration Structure

```typescript
interface RepositoryConfig {
  id: string
  name: string
  path: string                     // Local filesystem path
  useGitWorktrees?: boolean        // Enable git worktree isolation
  originalBranch?: string          // Base branch for worktrees (default: main)
  
  codeExecution: {
    type: 'local' | 'docker'       // Execution environment type
    setupCommands?: string         // Commands run during workspace setup
    serverCommand?: string         // Command to start dev server
    serverPortPlaceholder?: string // Placeholder for dynamic port assignment
    
    // Docker-specific settings
    image?: string                 // Docker image to use
    envVars?: Record<string, string>
    volumes?: string[]             // Additional volume mounts
    ports?: Record<number, number> // Port mappings
  }
  
  previewConfig?: {
    framework?: string             // Detected or configured framework
    serverStartCommand?: string    // Override server command
    serverPort?: number            // Fixed server port
    injectionStrategy?: {          // Script injection for debugging
      method: 'html-injection' | 'middleware'
      config: Record<string, any>
    }
  }
}
```

### 2. Workspace System

The workspace system provides isolated environments for code execution. It supports two implementations:

```mermaid
graph TB
    subgraph "Workspace Service"
        WS[WorkspaceService]
        WS --> CW[Create Workspace]
        WS --> GW[Get Workspace]
        WS --> SW[Setup Workspace]
        WS --> TW[Teardown Workspace]
    end
    
    subgraph "Workspace Types"
        BW[BaseWorkspace - Abstract]
        LW[LocalWorkspace]
        DW[DockerWorkspace]
        
        BW --> LW
        BW --> DW
    end
    
    subgraph "Workspace Lifecycle"
        NS[NOT_SETUP]
        SU[SETTING_UP]
        ID[IDLE]
        RU[RUNNING]
        
        NS --> SU
        SU --> ID
        ID --> RU
        RU --> ID
    end
    
    WS --> BW
    CW --> LW
    CW --> DW
```

#### Workspace Lifecycle

1. **NOT_SETUP**: Initial state, workspace not yet created
2. **SETTING_UP**: Creating worktree/container, running setup commands
3. **IDLE**: Ready for command execution
4. **RUNNING**: Actively executing commands or running preview server

#### LocalWorkspace Implementation

LocalWorkspace provides git-based isolation using worktrees or direct repository access:

```mermaid
graph LR
    subgraph "Git Worktree Mode"
        MR[Main Repository]
        WT1[Worktree Task-1]
        WT2[Worktree Task-2]
        WT3[Worktree Task-3]
        
        MR --> WT1
        MR --> WT2
        MR --> WT3
    end
    
    subgraph "Direct Mode"
        DR[Direct Repository]
        T1[Task 1]
        T2[Task 2]
        
        DR --> T1
        DR --> T2
    end
```

**Git Worktree Features:**
- **Isolation**: Each task gets its own worktree with separate working directory
- **Concurrency**: Multiple tasks can work on the same repository simultaneously
- **Branch Management**: Automatic branch creation/deletion
- **Location**: `{WORKTREE_DIR}/{repo-name}-workspace-{taskId}`
- **Branch Naming**: `workspace-{taskId}`

**Direct Mode Features:**
- **Simplicity**: Works directly in the repository path
- **No Isolation**: All tasks share the same working directory
- **Use Cases**: Single-task workflows, testing, development

#### DockerWorkspace Implementation

DockerWorkspace provides complete isolation using Docker containers:

```mermaid
graph TB
    subgraph "Docker Workspace"
        DC[Docker Container]
        DC --> WD[/workspace Directory]
        DC --> ENV[Environment Variables]
        DC --> VOL[Volume Mounts]
        
        subgraph "Operations"
            FO[File Operations]
            CO[Command Execution]
            SO[Server Operations]
        end
        
        DC --> FO
        DC --> CO
        DC --> SO
    end
    
    subgraph "Host System"
        HR[Host Repository]
        HD[Host Docker]
        
        HR -.->|Copy Files| WD
        HD --> DC
    end
```

**Docker Features:**
- **Complete Isolation**: Full container isolation from host
- **Reproducibility**: Consistent environment across systems
- **Security**: Sandboxed execution environment
- **Resource Control**: CPU/memory limits via Docker
- **Container Naming**: `moatless-task-{taskId}`

### 3. Preview Environment System

The preview environment system manages development servers for testing changes:

```mermaid
graph TB
    subgraph "Preview Environment Service"
        PES[PreviewEnvironmentService]
        PES --> SS[Start Server]
        PES --> ST[Stop Server]
        PES --> RS[Restart Server]
        PES --> GS[Get Status]
        PES --> GL[Get Logs]
    end
    
    subgraph "Server Management"
        SH[ServerHandler]
        SH --> PM[Process Management]
        SH --> PP[Port Assignment]
        SH --> LM[Log Management]
        SH --> SM[State Monitoring]
    end
    
    subgraph "Script Injection"
        SI[Script Injector]
        SI --> HI[HTML Injection]
        SI --> MW[Middleware Injection]
        SI --> FD[Framework Detection]
    end
    
    PES --> SH
    PES --> SI
```

#### Preview Environment Features

1. **Framework Detection**: Automatically detects Vite, Next.js, CRA, Webpack
2. **Script Injection**: Injects debugging/monitoring scripts into preview apps
3. **Process Recovery**: Persists server metadata for crash recovery
4. **Port Management**: Dynamic port assignment with conflict resolution
5. **Log Streaming**: Real-time server log access

#### Script Injection Strategy

The preview environment can inject monitoring scripts to enable:
- Console log capture
- Network request monitoring
- Error tracking
- Navigation tracking
- Element inspection

```typescript
interface ScriptInjectionStrategy {
  method: 'html-injection' | 'middleware'
  config: {
    indexPath?: string        // Path to HTML file
    insertLocation?: string   // Where to inject (head/body)
    middleware?: string       // Middleware type
  }
}
```

### 4. Git Worktree Architecture

Git worktrees provide lightweight, isolated working directories:

```mermaid
graph TB
    subgraph "Repository Structure"
        MR[Main Repository<br/>/repos/my-project]
        GD[.git Directory]
        WTD[Worktrees Directory<br/>/repos/.moatless-worktrees]
        
        MR --> GD
        GD --> WTD
    end
    
    subgraph "Worktree Creation"
        WT1[my-project-workspace-abc123]
        BR1[Branch: workspace-abc123]
        
        WT2[my-project-workspace-def456]
        BR2[Branch: workspace-def456]
        
        WTD --> WT1
        WTD --> WT2
        WT1 --> BR1
        WT2 --> BR2
    end
    
    subgraph "Worktree Lifecycle"
        CREATE[git worktree add]
        WORK[Modifications]
        MERGE[Merge Changes]
        CLEANUP[git worktree remove]
        
        CREATE --> WORK
        WORK --> MERGE
        MERGE --> CLEANUP
    end
```

#### Worktree Benefits

1. **Isolation**: Each task works in its own directory
2. **Performance**: Lightweight compared to full clones
3. **Disk Efficiency**: Shares object database with main repo
4. **Concurrent Tasks**: Multiple tasks on same repository
5. **Clean Rollback**: Easy to remove without affecting main repo

#### Worktree Configuration

Environment variables:
- `WORKTREE_DIR`: Base directory for all worktrees (default: `../`)
- Repository setting: `useGitWorktrees: true/false`

## Docker Worker Architecture

For Docker-based agent execution and containerized deployment scenarios, see the [Docker Worker Architecture](./docker-worker-architecture.md) documentation, which covers:

- Repository-based configuration for local or Docker agent execution
- Docker worker container management and event streaming
- Deployment strategies for development and production
- Security considerations and best practices

## Best Practices

### Repository Configuration

1. **Enable Worktrees** for multi-task repositories:
   ```json
   {
     "useGitWorktrees": true,
     "originalBranch": "main"
   }
   ```

2. **Use Docker** for untrusted code execution:
   ```json
   {
     "codeExecution": {
       "type": "docker",
       "image": "node:18"
     }
   }
   ```

3. **Configure Setup Commands** for dependency installation:
   ```json
   {
     "codeExecution": {
       "setupCommands": "npm install && npm run build"
     }
   }
   ```

### Workspace Management

1. **Graceful Teardown**: Always check for uncommitted changes
2. **Resource Cleanup**: Ensure containers/worktrees are removed
3. **Error Recovery**: Implement retry logic for transient failures
4. **Logging**: Enable comprehensive logging for debugging

### Preview Environments

1. **Framework Detection**: Let the system auto-detect when possible
2. **Port Management**: Use dynamic ports to avoid conflicts
3. **Script Injection**: Enable for development, disable for production
4. **Process Monitoring**: Implement health checks for preview servers

## Performance Considerations

### Worktree Performance

- **Creation Time**: ~1-5 seconds for typical repositories
- **Disk Usage**: Minimal (only working directory files)
- **Branch Operations**: Fast due to shared object database

### Docker Performance

- **Container Creation**: ~5-10 seconds including file copy
- **File Operations**: Slower due to docker exec overhead
- **Resource Usage**: Higher memory/CPU footprint

### Optimization Strategies

1. **Workspace Pooling**: Pre-create workspaces for common repositories
2. **Lazy Setup**: Defer expensive operations until needed
3. **Parallel Operations**: Run setup commands concurrently
4. **Caching**: Cache framework detection and dependency installation

## Monitoring and Debugging

### Workspace Metrics

- Workspace creation time
- Setup command duration
- Active workspace count
- Resource utilization

### Preview Environment Metrics

- Server start time
- Port allocation success rate
- Process recovery count
- Script injection success

### Debugging Tools

1. **Workspace Status API**: Check workspace state and capabilities
2. **Preview Logs**: Access server logs via API
3. **Process Validation**: Verify running processes
4. **Git Status**: Monitor repository state

## Future Enhancements

1. **Remote Workspaces**: Support for remote Docker hosts
2. **Workspace Templates**: Pre-configured workspace definitions
3. **Distributed Execution**: Multi-node workspace distribution
4. **Advanced Isolation**: User namespaces, rootless containers
5. **Workspace Snapshots**: Save/restore workspace state