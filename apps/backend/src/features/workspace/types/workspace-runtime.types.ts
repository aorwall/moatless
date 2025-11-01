import { dateSchema } from '@moatless/types'
import { z } from 'zod'

/**
 * Execution target configuration for workspace
 */
export const ExecutionTargetSchema = z.object({
  mode: z.literal('local'),
  workspacePath: z.string(),
  lastValidatedAt: dateSchema.optional(),
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
  startedAt: dateSchema.optional(),
})

export type AgentState = z.infer<typeof AgentStateSchema>

/**
 * Preview server state
 * Consolidated from preview_servers table to eliminate duplication
 */
export const PreviewStateSchema = z.object({
  status: z.enum(['stopped', 'starting', 'started', 'failed']),
  endpoint: z.string().optional(),
  proxyUrl: z.string().optional(),
  port: z.number().optional(),
  pid: z.number().optional(),
  inspectorEnabled: z.boolean().optional(),
  lastStartedAt: dateSchema.optional(),
  lastStoppedAt: dateSchema.optional(),
  lastError: z.string().optional(),
})

export type PreviewState = z.infer<typeof PreviewStateSchema>

/**
 * Lock for coordinating concurrent operations
 */
export const WorkspaceLockSchema = z.object({
  type: z.enum(['setup', 'preview', 'agent']),
  owner: z.string(),
  acquiredAt: dateSchema,
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
  createdAt: dateSchema,
  updatedAt: dateSchema,
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
  const now = new Date()
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
    updatedAt: new Date(),
  }
}
