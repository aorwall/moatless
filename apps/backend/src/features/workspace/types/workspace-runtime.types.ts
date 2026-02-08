import { SANDBOX_WORKSPACE_DIR } from '@moatless/sandbox-schemas'
import { dateSchema, EnvironmentStatusSchema } from '@moatless/types'
import { z } from 'zod'

/**
 * Standard workspace path inside Kubernetes pods
 * This is where repositories are cloned and where the working directory is set
 *
 * @deprecated Use SANDBOX_WORKSPACE_DIR from @moatless/sandbox-schemas instead
 */
export const CONTAINER_WORKSPACE_PATH = SANDBOX_WORKSPACE_DIR

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
export const ExecutionTargetSchema = z.object({
  mode: z.literal('kubernetes'),
  status: EnvironmentStatusSchema.optional(),
  previewHostPort: z.number().optional(),
  repositoryCloneUrl: z.string(),
  repositoryBranch: z.string().optional(),
  lastHealthCheck: dateSchema.optional(),
  lastError: z.string().optional(),
  servers: z.record(z.string(), z.number()).optional(),
  podName: z.string().optional(), // Not known until pod is created
  podNamespace: z.string(),
  serviceName: z.string(), // Stable DNS name — deterministic from taskId, known before pod exists
  podPhase: KubernetesPodPhaseSchema.optional(), // Raw K8s pod phase for detailed status
  sandboxPort: z.number().default(8001),
})

export type ExecutionTarget = z.infer<typeof ExecutionTargetSchema>

/**
 * Agent execution state (ephemeral — queried from sandbox, not persisted)
 */
export const AgentStateSchema = z.object({
  state: z.enum(['idle', 'running']),
  sessionId: z.string().optional(),
  agentType: z.string().optional(),
  startedAt: dateSchema.optional(),
})

export type AgentState = z.infer<typeof AgentStateSchema>

/**
 * Preview server state (ephemeral — queried from sandbox, not persisted)
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
 * Get workspace path from execution target
 * All container-based execution targets use the same standard path
 */
export function getWorkspacePathFromTarget(_target: ExecutionTarget): string {
  return CONTAINER_WORKSPACE_PATH
}

/**
 * Check if workspace is available based on execution target
 * Uses unified status field for consistent checking
 */
export function isWorkspaceAvailable(target: ExecutionTarget): boolean {
  return target.status === 'ready'
}

/**
 * Get sandbox API URL from execution target
 * Returns null if the sandbox URL is not available (e.g., pod not running)
 */
export function getSandboxUrl(target: ExecutionTarget): string | null {
  return `http://${target.serviceName}.${target.podNamespace}.svc.cluster.local:${target.sandboxPort}`
}
