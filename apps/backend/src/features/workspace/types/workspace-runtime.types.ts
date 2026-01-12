import { SANDBOX_WORKSPACE_DIR } from '@moatless/sandbox-schemas'
import { dateSchema } from '@moatless/types'
import { z } from 'zod'

/**
 * Standard workspace path inside Docker and Kubernetes containers/pods
 * This is where repositories are cloned and where the working directory is set
 *
 * @deprecated Use SANDBOX_WORKSPACE_DIR from @moatless/sandbox-schemas instead
 */
export const CONTAINER_WORKSPACE_PATH = SANDBOX_WORKSPACE_DIR

/**
 * Docker container status
 */
export const DockerContainerStatusSchema = z.enum([
  'created',
  'running',
  'exited',
  'removing',
  'not_created',
])

export type DockerContainerStatus = z.infer<typeof DockerContainerStatusSchema>

/**
 * Docker execution target configuration
 * Used when workspace runs inside a Docker container
 */
export const DockerExecutionTargetSchema = z.object({
  mode: z.literal('docker'),
  containerName: z.string(),
  containerId: z.string().optional(),
  containerStatus: DockerContainerStatusSchema.optional(),
  hostPort: z.number(), // Port mapped to localhost for HTTP communication
  repositoryCloneUrl: z.string(), // Git URL for container to clone
  repositoryBranch: z.string().optional(),
  workerPort: z.number().default(8001), // Internal container port
  lastHealthCheck: dateSchema.optional(),
  lastError: z.string().optional(),
})

export type DockerExecutionTarget = z.infer<typeof DockerExecutionTargetSchema>

/**
 * Kubernetes pod phases
 * Note: 'NotFound' is a custom phase indicating the pod doesn't exist in the cluster
 */
export const KubernetesPodPhaseSchema = z.enum([
  'Pending',
  'Running',
  'Succeeded',
  'Failed',
  'Unknown',
  'Terminating',
  'NotFound',
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
 * Discriminated union supporting docker and kubernetes modes
 */
export const ExecutionTargetSchema = z.discriminatedUnion('mode', [
  DockerExecutionTargetSchema,
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
  status: z.enum(['stopped', 'installing', 'starting', 'started', 'failed']),
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
  snapshotRevision: z.string(),
  executionTarget: ExecutionTargetSchema,
  agent: AgentStateSchema.optional(),
  preview: PreviewStateSchema,
  locks: z.array(WorkspaceLockSchema).optional(),
  createdAt: dateSchema,
  updatedAt: dateSchema,
})

export type WorkspaceRuntimeSnapshot = z.infer<typeof WorkspaceRuntimeSnapshotSchema>

/**
 * Configuration for creating a Docker workspace snapshot
 */
export interface DockerWorkspaceConfig {
  taskId: string
  repositoryId: string
  containerName: string
  containerId?: string
  containerStatus?: DockerContainerStatus
  hostPort: number
  repositoryCloneUrl: string
  repositoryBranch?: string
  workerPort?: number
  lastError?: string
}

/**
 * Create a new Docker workspace runtime snapshot
 */
export function createDockerWorkspaceSnapshot(
  config: DockerWorkspaceConfig,
): WorkspaceRuntimeSnapshot {
  const now = new Date()
  const executionTarget: DockerExecutionTarget = {
    mode: 'docker',
    containerName: config.containerName,
    hostPort: config.hostPort,
    repositoryCloneUrl: config.repositoryCloneUrl,
    workerPort: config.workerPort ?? 8001,
  }
  if (config.containerId) {
    executionTarget.containerId = config.containerId
  }
  if (config.containerStatus) {
    executionTarget.containerStatus = config.containerStatus
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
    snapshotRevision: '1',
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
    snapshotRevision: '1',
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
 * Create a workspace runtime snapshot from an execution target.
 */
export function createWorkspaceSnapshotFromTarget(
  taskId: string,
  repositoryId: string,
  executionTarget: ExecutionTarget,
): WorkspaceRuntimeSnapshot {
  const now = new Date()
  return {
    taskId,
    repositoryId,
    snapshotRevision: '1',
    executionTarget: { ...executionTarget },
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
 * Update the snapshot revision (for optimistic locking)
 */
export function bumpSnapshotRevision(snapshot: WorkspaceRuntimeSnapshot): WorkspaceRuntimeSnapshot {
  return {
    ...snapshot,
    snapshotRevision: String(Number.parseInt(snapshot.snapshotRevision) + 1),
    updatedAt: new Date(),
  }
}

/**
 * Type guard to check if execution target is Docker
 */
export function isDockerExecutionTarget(target: ExecutionTarget): target is DockerExecutionTarget {
  return target.mode === 'docker'
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
 * Get workspace path from execution target
 * All container-based execution targets use the same standard path
 */
export function getWorkspacePathFromTarget(_target: ExecutionTarget): string {
  // Both Docker and Kubernetes use the same workspace path
  return CONTAINER_WORKSPACE_PATH
}

/**
 * Check if workspace is available based on execution target
 */
export function isWorkspaceAvailable(target: ExecutionTarget): boolean {
  if (isDockerExecutionTarget(target)) {
    return target.containerStatus === 'running'
  }
  // Kubernetes: available if pod is running
  return target.podPhase === 'Running'
}
