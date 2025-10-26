import { z } from 'zod'

/**
 * Execution target configuration for workspace
 */
export const ExecutionTargetSchema = z.object({
  mode: z.literal('local'),
  workspacePath: z.string(),
  lastValidatedAt: z.string().datetime().optional(),
  lastError: z.string().optional(),
})

export type ExecutionTarget = z.infer<typeof ExecutionTargetSchema>

/**
 * Agent execution state
 */
export const AgentStateSchema = z.object({
  state: z.enum(['idle', 'running']),
  sessionId: z.string().optional(),
  agentType: z.string().optional(),
  startedAt: z.string().datetime().optional(),
})

export type AgentState = z.infer<typeof AgentStateSchema>

/**
 * Preview server state
 */
export const PreviewStateSchema = z.object({
  status: z.enum(['stopped', 'starting', 'started', 'failed']),
  endpoint: z.string().optional(),
  proxyUrl: z.string().optional(),
  lastStartedAt: z.string().datetime().optional(),
  lastStoppedAt: z.string().datetime().optional(),
  lastError: z.string().optional(),
})

export type PreviewState = z.infer<typeof PreviewStateSchema>

/**
 * Lock for coordinating concurrent operations
 */
export const WorkspaceLockSchema = z.object({
  type: z.enum(['setup', 'preview', 'agent']),
  owner: z.string(),
  acquiredAt: z.string().datetime(),
})

export type WorkspaceLock = z.infer<typeof WorkspaceLockSchema>

/**
 * Complete runtime snapshot for a workspace
 * This persists all workspace state and survives process restarts
 */
export const WorkspaceRuntimeSnapshotSchema = z.object({
  taskId: z.string(),
  repositoryId: z.string(),
  configRevision: z.string(),
  executionTarget: ExecutionTargetSchema,
  agent: AgentStateSchema.optional(),
  preview: PreviewStateSchema,
  locks: z.array(WorkspaceLockSchema).optional(),
  createdAt: z.string().datetime(),
  updatedAt: z.string().datetime(),
})

export type WorkspaceRuntimeSnapshot = z.infer<typeof WorkspaceRuntimeSnapshotSchema>

/**
 * Create a new workspace runtime snapshot with defaults
 */
export function createWorkspaceSnapshot(
  taskId: string,
  repositoryId: string,
  workspacePath: string,
): WorkspaceRuntimeSnapshot {
  const now = new Date().toISOString()
  return {
    taskId,
    repositoryId,
    configRevision: '1',
    executionTarget: {
      mode: 'local',
      workspacePath,
      lastValidatedAt: now,
    },
    preview: {
      status: 'stopped',
    },
    createdAt: now,
    updatedAt: now,
  }
}

/**
 * Update the config revision (for optimistic locking)
 */
export function bumpConfigRevision(snapshot: WorkspaceRuntimeSnapshot): WorkspaceRuntimeSnapshot {
  return {
    ...snapshot,
    configRevision: String(Number.parseInt(snapshot.configRevision) + 1),
    updatedAt: new Date().toISOString(),
  }
}
