# Workspace System Implementation Guide

> **LLM Code Generation Document**: This document provides implementation guidance for code generation LLMs working with the Moatless workspace system. It contains specific implementation details, method signatures, file paths, and executable patterns for creating, managing, and operating workspaces.

## Quick Implementation Reference

### Key Files and Methods
- **Workspace Service**: `apps/backend/services/workspace.service.ts`
  - `createWorkspace()` (lines 30-84) - Creates workspace with file space and environment
  - `getOrCreateWorkspace()` (lines 308-370) - Gets existing or creates new workspace
  - `cleanupWorkspace()` (lines 560-590) - Environment cleanup only
  - `removeWorktree()` (lines 241-258) - Worktree removal for task close/delete
- **Unified Workspace**: `apps/backend/workspaces/workspace.ts`
  - `setup()` (line 74) - Environment setup delegation
  - `runCommand()` (line 194) - Command execution delegation
  - File operations: `readFile()`, `writeFile()`, `getGitDiff()` (lines 150-188)
- **Docker Environment**: `apps/backend/environments/docker-environment.ts`
  - `setup()` (lines 78-157) - Container creation and setup
  - `runCommand()` (lines 308-357) - Direct docker exec command execution
- **Local Environment**: `apps/backend/environments/local-environment.ts`
  - `setup()` (lines 64-105) - Path validation and setup commands
  - `runCommand()` (lines 139-169) - Direct process execution

### Environment State Values
- `'not_setup'` - Initial state, no setup attempted
- `'initializing'` - Setup in progress (`setupInProgress = true`)
- `'ready'` - Setup complete (`setupComplete = true`)
- `'error'` - Setup failed (`_error` set)

### Configuration Schema
```typescript
// Repository configuration
{
  path: string,                    // Repository path
  useGitWorktrees: boolean,        // Enable git worktree isolation
  codeExecution: {
    type: 'local' | 'docker',     // Environment type
    setupCommands: string[],       // Commands to run during setup
    dockerImage?: string,          // Docker image (default: moatless-vibe-universal:latest)
    envVars?: Record<string, string>
  }
}
```

## Workspace Architecture: File Space vs Environment

Workspaces consist of two separate, composable components:

### 1. File Space - Where Code Lives
**Implementation**: `apps/backend/services/file-operations.service.ts`
- Handles all file system operations (read, write, list, search)
- Manages git operations (diff, status, changes, commit hash)
- Provides path validation and security
- Location determined by `repository.useGitWorktrees` setting

### 2. Environment - Where Code Executes
**Implementation**: `apps/backend/environments/` (base, local, docker)
- Handles command execution
- Manages setup process and lifecycle
- Provides isolated execution context
- Type determined by `repository.codeExecution.type`

### Component Composition
```typescript
// Workspace composes both components
class Workspace {
  constructor(
    private fileOperations: FileOperations,  // File space operations
    private environment: BaseEnvironment     // Execution environment
  ) {}
  
  // File operations delegate to FileOperations
  async readFile(path: string) {
    return this.fileOperations.readFile(path)
  }
  
  // Command execution delegates to Environment
  async runCommand(command: string) {
    return this.environment.runCommand(command)
  }
}
```

## File Space: Git Worktrees vs Repository Path

### Configuration: `repository.useGitWorktrees`

#### Worktree Mode (`useGitWorktrees: true`)
**Implementation**: `apps/backend/services/workspace.service.ts:96-113`

**Behavior**:
- Creates isolated git worktree at `{worktreeDir}/{repoBaseName}-workspace-{taskId}`
- Enables multiple concurrent tasks on different branches
- Automatic worktree creation/recreation if path missing
- Branch isolation prevents conflicts between tasks

**Path Resolution**:
```typescript
// Worktree directory priority
const worktreeDir = process.env.WORKTREE_DIR || path.join(repo.path, '..')
const workspacePath = path.join(worktreeDir, `${repoBaseName}-workspace-${taskId}`)
```

**Branch Management**:
1. Use explicit `branchName` parameter if provided
2. Use last completed iteration branch via `getLastIterationBranch()`
3. Create new branch `workspace-{taskId}`

#### Direct Repository Mode (`useGitWorktrees: false`)
**Implementation**: `apps/backend/services/workspace.service.ts:96-99`

**Behavior**:
- Uses original repository path directly: `return repo.path`
- Single task per repository limitation
- No branch isolation
- Direct working directory modifications

### Worktree Creation Process
**Implementation**: `apps/backend/services/workspace.service.ts:160-195`

```typescript
// Validation and recreation logic
const pathExists = await this.pathExists(workspacePath)
const isValidWorktree = pathExists && (await isGitWorktree(workspacePath))

if (!isValidWorktree) {
  // Recreate worktree
  await createGitWorktree(repo.path, workspacePath, branchName, newBranchName)
}
```

## Complete Workspace Setup Flow

### 1. Workspace Creation
**Entry Point**: `WorkspaceService.createWorkspace()` (lines 30-84)

**Process**:
1. **Repository Validation**: Load repository config, validate path exists
2. **File Space Creation**: 
   - If `useGitWorktrees: true` → Create/ensure worktree at isolated path
   - If `useGitWorktrees: false` → Use repository path directly
3. **Environment Selection**: Create environment based on `codeExecution.type`
4. **Workspace Composition**: Create unified workspace with file operations and environment
5. **Registration**: Store in `activeWorkspaces` map

### 2. Environment Setup
**Entry Point**: `workspace.setup()` → `environment.setup()`

**LocalEnvironment Setup** (lines 64-105):
```typescript
// Path validation
if (!(await this.pathExists(this.workspacePath))) {
  throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
}

// Background setup commands
this.runSetupCommandsInBackground()
```

**DockerEnvironment Setup** (lines 78-157):
```typescript
// Port allocation
const workerPort = await this.portAllocationService.allocatePort(taskId, 'worker')
const previewPort = await this.portAllocationService.allocatePort(taskId, 'preview')

// Container creation and start
await this.createContainer()
await this.startContainer()
await this.waitForWorkerReady()

// Setup commands execution
for (const command of setupCommands) {
  await this.runCommand(command, 300000) // 5 minute timeout
}
```

### 3. Command Execution
**Entry Point**: `workspace.runCommand()` → `environment.runCommand()`

Commands execute in the environment's context but operate on the file space.

## Environment State Transitions

### State Management
**Implementation**: `apps/backend/environments/base-environment.ts` (getStatus method)

```typescript
getStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' {
  if (this._error) return 'error'
  if (this.setupInProgress) return 'initializing'  
  if (this.setupComplete) return 'ready'
  return 'not_setup'
}
```

### State Transitions

#### LocalEnvironment States
1. **not_setup** → **initializing**: `setup()` called, path validation begins
2. **initializing** → **ready**: Path valid, setup commands started (background)
3. **initializing** → **error**: Path validation fails

#### DockerEnvironment States  
1. **not_setup** → **initializing**: `setup()` called, `setupInProgress = true`
2. **initializing** → **ready**: Container created, started, worker ready, setup commands complete
3. **initializing** → **error**: Any step fails, `_error` set, `setupInProgress = false`

### State Checking
```typescript
// Check environment state
const status = workspace.getStatus() // Delegates to environment.getStatus()

// State-based capability checks
const canExecute = workspace.canExecuteCommands() // Only when environment is ready
```

## Docker Environment Setup

### Container Creation Process
**Implementation**: `apps/backend/environments/docker-environment.ts:174-256`

#### 1. Port Allocation
```typescript
// Allocate host ports
const workerPort = await this.portAllocationService.allocatePort(taskId, 'worker')
const previewPort = await this.portAllocationService.allocatePort(taskId, 'preview')

// Container port mapping
this.workerPortMapping = { containerPort: 8001, hostPort: workerPort }
this.previewPortMapping = { containerPort: 3000, hostPort: previewPort }
```

#### 2. Container Configuration
```typescript
// Container creation command
const createCommand = `docker create --name "${containerName}"
  -v "${workspacePath}:/repo"
  -v "${dataDir}:/data"
  -p ${workerPort}:8001
  -p ${previewPort}:3000
  -e "WORKER_MODE=true"
  -e "WORKSPACE_DIR=/repo"
  -e "PORT=8001"
  ${dockerImage}`
```

#### 3. Container Lifecycle
```typescript
// Container start sequence
await this.createContainer()     // Create container with volumes and ports
await this.startContainer()      // Start the container
await this.waitForWorkerReady()  // Wait for worker health check
```

### Command Execution via Docker Exec
**Implementation**: `apps/backend/environments/docker-environment.ts:308-357`

```typescript
async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
  // Direct docker exec execution
  const dockerCommand = `docker exec -w /repo ${this.containerId} /bin/bash -c "${command.replace(/"/g, '\\"')}"`
  
  const { stdout, stderr } = await execAsync(dockerCommand, { timeout })
  
  return {
    exitCode: 0,
    output: stdout + stderr,
    durationMs: Date.now() - startTime,
    command
  }
}
```

### Container Configuration Details

#### Volume Mounting
- **Workspace**: `{workspacePath}:/repo` (working directory)
- **Data**: `{dataDir}:/data` (persistent data)
- **Claude Config**: `{claudeConfigDir}:/home/ubuntu/.claude` (authentication)
- **Git Worktree Support**: When `useGitWorktrees: true`, mounts main repo `.git` directory at `/main-repo-git:ro`

##### Git Worktree Integration
When using git worktrees with Docker environments, the system automatically handles the path mismatch between host and container:

1. **Mount Strategy**: The main repository's `.git` directory is mounted read-only at `/main-repo-git`
2. **Environment Variables**: Git commands use `GIT_DIR` and `GIT_WORK_TREE` environment variables instead of modifying files
3. **Automatic Configuration**: The Docker entrypoint detects worktrees and sets:
   - `GIT_DIR=/main-repo-git/worktrees/{worktree-name}`
   - `GIT_WORK_TREE=/repo`
4. **Compatibility**: The worktree remains valid both inside the container and on the host system

This approach ensures git commands work correctly in Docker containers without breaking the worktree for the host system.

#### Environment Variables
```typescript
// Container environment
WORKER_MODE=true
WORKSPACE_DIR=/repo
DATA_DIR=/data
PORT=8001
PREVIEW_PORT=3000
WORKER_TASK_ID=${taskId}
```

#### Container Specifications
- **Base Image**: `moatless-vibe-universal:latest` (configurable via `dockerImage`)
- **Working Directory**: `/repo`
- **Home Directory**: `/repo` (for user cache and config)
- **User**: `ubuntu` (UID 1000)
- **Required Tools**: `pnpm`, `node`, `python`, `git`, `bash`

### Setup Commands Execution
**Implementation**: `apps/backend/environments/docker-environment.ts:130-147`

```typescript
// Run each setup command with timeout
for (const command of setupCommands) {
  const result = await this.runCommand(command, 300000) // 5 minutes
  
  if (result.exitCode !== 0) {
    this.logger.warn(`Setup command failed: ${command}`, {
      exitCode: result.exitCode,
      output: result.output
    })
  }
}
```

## Local Environment Setup

### Path Validation and Setup
**Implementation**: `apps/backend/environments/local-environment.ts:76-91`

```typescript
// Validate workspace path exists
if (!(await this.pathExists(this.workspacePath))) {
  throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
}

// Run setup commands in background
this.runSetupCommandsInBackground(startTime).catch((error) => {
  this._error = error.message
})
```

### Command Execution
**Implementation**: `apps/backend/environments/local-environment.ts:139-169`

```typescript
async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
  // Direct process execution in workspace directory
  const { stdout, stderr } = await execAsync(command, {
    cwd: this.workspacePath,
    timeout
  })
  
  return {
    exitCode: 0,
    output: stdout + stderr,
    durationMs: Date.now() - startTime,
    command
  }
}
```

## Technical Reference

### API Method Signatures

#### WorkspaceService Methods
```typescript
// Workspace creation
createWorkspace(taskId: string, repositoryId: string, branchName?: string): Promise<Workspace>

// Workspace retrieval/creation
getOrCreateWorkspace(taskId: string, repositoryId: string, branchName?: string): Promise<Workspace>

// Environment cleanup (processes only)
cleanupWorkspace(taskId: string): Promise<void>

// Worktree removal (file system)
removeWorktree(taskId: string): Promise<void>

// Capability checks
canExecuteCommands(taskId: string): Promise<boolean>
canReadWriteFiles(taskId: string): Promise<boolean>
```

#### Environment Methods
```typescript
// Setup and lifecycle
setup(iterationNumber?: number): Promise<void>
teardown(force?: boolean): Promise<void>
getStatus(): 'not_setup' | 'initializing' | 'ready' | 'error'

// Command execution
runCommand(command: string, timeout?: number): Promise<CommandResult>
isRunning(): boolean
```

#### File Operations
```typescript
// File system operations
readFile(relativePath: string): Promise<string>
writeFile(relativePath: string, content: string): Promise<void>
listFiles(relativePath?: string): Promise<string[]>
searchFiles(query: string): Promise<FileSearchResponse>

// Git operations
getGitDiff(): Promise<GitDiffResponse>
getGitStatus(): Promise<GitStatus>
getGitChanges(): Promise<GitChanges>
getCurrentCommitHash(): Promise<string>
```

### Command Result Structure
```typescript
interface CommandResult {
  exitCode: number
  output: string
  durationMs: number
  command: string
}
```

### Environment Configuration
```typescript
// Docker environment config
{
  type: 'docker',
  dockerImage: 'moatless-vibe-universal:latest',
  setupCommands: ['pnpm install', 'pnpm build'],
  envVars: {
    NODE_ENV: 'production',
    PORT: '3000'
  }
}

// Local environment config
{
  type: 'local',
  setupCommands: ['npm install', 'npm run build']
}
```

## Implementation Patterns

### Workspace Creation Pattern
```typescript
// Standard workspace creation
const workspace = await workspaceService.createWorkspace(taskId, repositoryId, branchName)
await workspace.setup()

// Check if ready for command execution
if (workspace.getStatus() === 'ready') {
  const result = await workspace.runCommand('echo "Hello World"')
}
```

### Error Handling Pattern
```typescript
try {
  const workspace = await workspaceService.getOrCreateWorkspace(taskId, repositoryId)
  await workspace.setup()
} catch (error) {
  if (error instanceof WorkspaceError) {
    // Handle workspace-specific errors
    logger.error('Workspace setup failed', { error: error.message, taskId })
  }
  throw error
}
```

### Environment State Monitoring
```typescript
// Monitor environment setup
const checkStatus = async () => {
  const status = workspace.getStatus()
  switch (status) {
    case 'not_setup':
      await workspace.setup()
      break
    case 'initializing':
      // Wait for setup completion
      await new Promise(resolve => setTimeout(resolve, 1000))
      return checkStatus()
    case 'ready':
      return true
    case 'error':
      throw new Error(`Environment setup failed: ${workspace.getError()}`)
  }
}
```

## Troubleshooting Guide

### Common Issues and Solutions

#### Workspace Path Issues
| Issue | Root Cause | Solution |
|-------|------------|----------|
| "Workspace path does not exist" | Worktree removed externally | Call `getOrCreateWorkspace()` - auto-recreates |
| "Branch 'X' does not exist" | Branch deleted from repository | Check git history, use different branch |
| "Container not created" | Docker environment setup failed | Check Docker daemon, image availability |

#### Environment State Issues
| State | Problem | Action |
|-------|---------|---------|
| `error` | Setup failed | Check logs, verify prerequisites |
| `initializing` (stuck) | Setup hanging | Check setup commands, restart |
| `not_setup` | Never initialized | Call `workspace.setup()` |

#### Docker-Specific Issues
| Issue | Cause | Solution |
|-------|-------|---------|
| "No such container" | Container not created | Check `docker ps -a`, recreate workspace |
| "Port already in use" | Port conflict | Use `PortAllocationService` or change ports |
| "pnpm command not found" | Missing tools in image | Use `moatless-vibe-universal:latest` |
| "fatal: not a git repository" | Git worktree path mismatch | Ensure main repo `.git` is mounted and env vars are set |

#### Command Execution Issues
| Issue | Environment | Solution |
|-------|-------------|---------|
| Command timeout | Both | Increase timeout parameter |
| Permission denied | Docker | Check user permissions in container |
| Working directory error | Docker | Ensure `/repo` volume mount |

### Debugging Commands
```bash
# Check Docker environment
docker ps -a | grep moatless
docker logs <container-id>
docker exec -it <container-id> /bin/bash

# Check worktree status
git worktree list
git worktree prune

# Check environment state
curl http://localhost:<worker-port>/health
```

### Recovery Procedures

#### Environment Recovery
```typescript
// Force cleanup and recreation
await workspaceService.cleanupWorkspace(taskId)
const workspace = await workspaceService.createWorkspace(taskId, repositoryId)
await workspace.setup()
```

#### Worktree Recovery
```typescript
// Remove and recreate worktree
await workspaceService.removeWorktree(taskId)
await workspaceService.cleanupWorkspace(taskId)
const workspace = await workspaceService.getOrCreateWorkspace(taskId, repositoryId)
```

This guide provides LLMs with comprehensive implementation details for working with the Moatless workspace system, including specific code patterns, error handling, and troubleshooting procedures.