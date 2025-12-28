import { dateSchema } from '@moatless/types'
import { z } from 'zod'

/**
 * Local execution target configuration
 */
export const LocalExecutionTargetSchema = z.object({
  mode: z.literal('local'),
  workspacePath: z.string(),
  isAvailable: z.boolean().optional(), // false when workspace path doesn't exist
  lastValidatedAt: dateSchema.optional(),
  lastError: z.string().optional(),
})

export type LocalExecutionTarget = z.infer<typeof LocalExecutionTargetSchema>

/**
 * Kubernetes pod phases
 */
export const KubernetesPodPhaseSchema = z.enum([
  'Pending',
  'Running',
  'Succeeded',
  'Failed',
  'Unknown',
])

export type KubernetesPodPhase = z.infer<typeof KubernetesPodPhaseSchema>

/**
 * Kubernetes execution target configuration
 * Used when workspace runs inside a Kubernetes pod
 */
export const KubernetesExecutionTargetSchema = z.object({
  mode: z.literal('kubernetes'),
  podName: z.string(),
  podNamespace: z.string(),
  podIp: z.string().optional(), // Available once pod is running
  podPhase: KubernetesPodPhaseSchema.optional(),
  repositoryCloneUrl: z.string(), // Git URL for pod to clone
  repositoryBranch: z.string().optional(),
  workerPort: z.number().default(8001),
  lastHealthCheck: dateSchema.optional(),
  lastError: z.string().optional(),
})

export type KubernetesExecutionTarget = z.infer<typeof KubernetesExecutionTargetSchema>

/**
 * Execution target configuration for workspace
 * Discriminated union supporting local and kubernetes modes
 */
export const ExecutionTargetSchema = z.discriminatedUnion('mode', [
  LocalExecutionTargetSchema,
  KubernetesExecutionTargetSchema,
])

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
  dirtyContext: z.boolean().optional(),
  needsSetup: z.boolean().optional(),
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
 * Create a new local workspace runtime snapshot with defaults
 */
export function createWorkspaceSnapshot(
  taskId: string,
  repositoryId: string,
  workspacePath: string,
  isAvailable = true,
): WorkspaceRuntimeSnapshot {
  const now = new Date()
  return {
    taskId,
    repositoryId,
    configRevision: '1',
    executionTarget: {
      mode: 'local',
      workspacePath,
      isAvailable,
      lastValidatedAt: now,
    },
    preview: {
      status: 'stopped',
      dirtyContext: false,
      needsSetup: true,
    },
    createdAt: now,
    updatedAt: now,
  }
}

/**
 * Configuration for creating a Kubernetes workspace snapshot
 */
export interface KubernetesWorkspaceConfig {
  taskId: string
  repositoryId: string
  podName: string
  podNamespace: string
  podIp?: string
  podPhase?: KubernetesPodPhase
  repositoryCloneUrl: string
  repositoryBranch?: string
  workerPort?: number
  lastError?: string
}

/**
 * Create a new Kubernetes workspace runtime snapshot
 */
export function createKubernetesWorkspaceSnapshot(
  config: KubernetesWorkspaceConfig,
): WorkspaceRuntimeSnapshot {
  const now = new Date()
  const executionTarget: KubernetesExecutionTarget = {
    mode: 'kubernetes',
    podName: config.podName,
    podNamespace: config.podNamespace,
    repositoryCloneUrl: config.repositoryCloneUrl,
    workerPort: config.workerPort ?? 8001,
  }
  if (config.podIp) {
    executionTarget.podIp = config.podIp
  }
  if (config.podPhase) {
    executionTarget.podPhase = config.podPhase
  }
  if (config.repositoryBranch) {
    executionTarget.repositoryBranch = config.repositoryBranch
  }
  if (config.lastError) {
    executionTarget.lastError = config.lastError
  }

  return {
    taskId: config.taskId,
    repositoryId: config.repositoryId,
    configRevision: '1',
    executionTarget,
    preview: {
      status: 'stopped',
      dirtyContext: false,
      needsSetup: true,
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

/**
 * Type guard to check if execution target is local
 */
export function isLocalExecutionTarget(target: ExecutionTarget): target is LocalExecutionTarget {
  return target.mode === 'local'
}

/**
 * Type guard to check if execution target is Kubernetes
 */
export function isKubernetesExecutionTarget(
  target: ExecutionTarget,
): target is KubernetesExecutionTarget {
  return target.mode === 'kubernetes'
}

/**
 * Get workspace path from execution target (only valid for local mode)
 * For Kubernetes mode, returns a standard path inside the pod
 */
export function getWorkspacePathFromTarget(target: ExecutionTarget): string {
  if (isLocalExecutionTarget(target)) {
    return target.workspacePath
  }
  // Kubernetes pods use a standard workspace path
  return '/workspace'
}

/**
 * Check if workspace is available based on execution target
 */
export function isWorkspaceAvailable(target: ExecutionTarget): boolean {
  if (isLocalExecutionTarget(target)) {
    return target.isAvailable !== false
  }
  // Kubernetes: available if pod is running
  return target.podPhase === 'Running'
}
