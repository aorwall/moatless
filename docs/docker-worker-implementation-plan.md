# Docker Worker Mode Implementation Plan

## Executive Summary

This document outlines the implementation plan for adding "Docker Worker Mode" to the backend, enabling agent execution in isolated Docker containers while reusing the existing `LocalWorkspace` functionality.

## Current State Analysis

### What Exists
1. **LocalWorkspace** - Fully functional workspace implementation that:
   - Manages git worktrees
   - Executes agents locally
   - Handles file I/O operations
   - Emits events through EventService

2. **DockerWorkspace** - Basic Docker container management that:
   - Creates and manages Docker containers
   - Executes commands via `docker exec`
   - Does NOT support agent execution (`triggerAgent` throws error)

3. **WorkspaceService** - Routes to appropriate workspace based on repository config

### Key Gap
The `DockerWorkspace` class cannot execute agents. The proposed solution is to run the entire backend in a "worker mode" inside Docker containers, where it uses `LocalWorkspace` internally.

## Implementation Plan

### Phase 1: Core Worker Mode Infrastructure (Week 1)

#### 1.1 Create Worker Mode Module
**File**: `apps/backend/src/worker-mode.ts`

```typescript
export interface WorkerModeConfig {
  apiUrl: string
  apiKey: string
  workspaceDir: string
}

export async function startWorkerMode(config: WorkerModeConfig): Promise<void> {
  // Initialize minimal services
  // Start HTTP server for task execution
  // Handle health checks
}
```

#### 1.2 Create Remote Services
**Files**:
- `apps/backend/src/services/remote-event.service.ts`
- `apps/backend/src/services/remote-storage.service.ts`

These services proxy calls to the main backend via HTTP.

#### 1.3 Update Main Entry Point
**File**: `apps/backend/src/index.ts`

Add logic to detect `WORKER_MODE` environment variable and route to appropriate startup.

### Phase 2: Worker Management System (Week 2)

#### 2.1 Create Worker Service
**File**: `apps/backend/src/services/worker.service.ts`

Manages Docker containers running in worker mode:
- Container lifecycle (create, start, stop, remove)
- Worker pool management
- Task delegation
- Health monitoring

#### 2.2 Add Worker API Endpoints
**File**: `apps/backend/src/routes/worker.routes.ts`

```typescript
// Worker event forwarding
app.post('/api/v1/worker/events', authenticateWorker, handleWorkerEvent)

// Worker registration
app.post('/api/v1/worker/register', handleWorkerRegistration)

// Worker health check
app.get('/api/v1/worker/health/:workerId', getWorkerHealth)
```

#### 2.3 Update Repository Types
**File**: `packages/shared/types/repository.ts`

Add worker configuration to repository settings:
```typescript
interface CodeExecutionConfig {
  type: 'local' | 'docker'
  useWorkers?: boolean  // New flag
  workerImage?: string
  workerResources?: {
    memory: string
    cpu: string
  }
}
```

### Phase 3: Integration & Testing (Week 3)

#### 3.1 Update Agent Service
**File**: `apps/backend/src/services/agent.service.ts`

Add logic to detect when to delegate to workers based on repository configuration.

#### 3.2 Docker Configuration
**Files**:
- Update `Dockerfile` to support worker mode
- Create `docker-compose.worker.yml` for testing

#### 3.3 Authentication & Security
- Implement worker API key generation
- Add authentication middleware
- Ensure network isolation

### Phase 4: Testing & Documentation (Week 4)

#### 4.1 Unit Tests
- Test worker mode initialization
- Test remote service implementations
- Test worker service logic

#### 4.2 Integration Tests
- Test full agent execution flow via workers
- Test worker failure scenarios
- Test concurrent executions

#### 4.3 Documentation Updates
- Update API documentation
- Create deployment guide
- Add troubleshooting section

## File Changes Summary

### New Files to Create
1. `apps/backend/src/worker-mode.ts` - Worker mode entry point
2. `apps/backend/src/services/remote-event.service.ts` - Event forwarding
3. `apps/backend/src/services/remote-storage.service.ts` - Storage proxy
4. `apps/backend/src/services/worker.service.ts` - Worker management
5. `apps/backend/src/routes/worker.routes.ts` - Worker API endpoints
6. `apps/backend/src/middleware/worker-auth.ts` - Worker authentication

### Files to Modify
1. `apps/backend/src/index.ts` - Add worker mode detection
2. `apps/backend/src/services/agent.service.ts` - Add worker delegation
3. `packages/shared/types/repository.ts` - Add worker config types
4. `apps/backend/Dockerfile` - Ensure git and workspace directory
5. `docker-compose.yml` - Add worker service example

### Files to Keep As-Is
1. `apps/backend/workspaces/local.ts` - No changes needed
2. `apps/backend/workspaces/docker.ts` - Keep for basic Docker operations

## Migration Strategy

### Stage 1: Silent Release
- Deploy worker mode without using it
- Test with internal repositories only
- Monitor performance and stability

### Stage 2: Opt-in Beta
- Add `useWorkers` flag to repository config
- Allow users to opt-in per repository
- Gather feedback and metrics

### Stage 3: Gradual Rollout
- Enable by default for new repositories
- Migrate existing Docker-based repositories
- Provide opt-out mechanism

### Stage 4: Full Migration
- Make worker mode the default for all Docker execution
- Deprecate direct DockerWorkspace agent execution
- Remove legacy code paths

## Risk Mitigation

### Performance Risks
- **Risk**: Overhead from HTTP communication
- **Mitigation**: Implement request batching and connection pooling

### Reliability Risks
- **Risk**: Worker container crashes
- **Mitigation**: Implement health checks and automatic restart

### Security Risks
- **Risk**: Unauthorized worker access
- **Mitigation**: Strong API key authentication and network isolation

## Success Metrics

1. **Performance**
   - Worker startup time < 5 seconds
   - Event forwarding latency < 100ms
   - Resource overhead < 10%

2. **Reliability**
   - Worker uptime > 99.9%
   - Successful task completion > 99%
   - Automatic recovery from failures

3. **Scalability**
   - Support 10+ concurrent workers
   - Linear scaling with worker count
   - Efficient resource utilization

## Timeline

- **Week 1**: Core infrastructure (Phase 1)
- **Week 2**: Worker management (Phase 2)
- **Week 3**: Integration and testing (Phase 3)
- **Week 4**: Documentation and deployment (Phase 4)
- **Week 5-6**: Beta testing and feedback
- **Week 7-8**: Production rollout

## Next Steps

1. Review and approve this implementation plan
2. Create detailed technical specifications
3. Set up development environment
4. Begin Phase 1 implementation
5. Schedule weekly progress reviews