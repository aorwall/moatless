import { SANDBOX_WORKSPACE_DIR } from '@moatless/sandbox-schemas'
import { dateSchema, EnvironmentStatusSchema } from '@moatless/types'
import { z } from 'zod'

/**
 * Standard workspace path inside Docker and Kubernetes containers/pods
 * This is where repositories are cloned and where the working directory is set
 *
 * @deprecated Use SANDBOX_WORKSPACE_DIR from @moatless/sandbox-schemas instead
 */
export const CONTAINER_WORKSPACE_PATH = SANDBOX_WORKSPACE_DIR

/**
 * Shared base fields for all execution targets
 */
const BaseExecutionTargetSchema = z.object({
  status: EnvironmentStatusSchema.optional(),
  previewHostPort: z.number().optional(),
  repositoryCloneUrl: z.string(),
  repositoryBranch: z.string().optional(),
  lastHealthCheck: dateSchema.optional(),
  lastError: z.string().optional(),
  servers: z.record(z.string(), z.number()).optional(),
})

/**
 * Docker execution target configuration
 * Used when workspace runs inside a Docker container
 *
 * Communication options:
 * - From within Docker network: use container name (e.g., http://container-name:8001)
 * - From host machine: use sandboxApiHostPort (e.g., http://localhost:59234)
 *
 * Port mappings:
 * - previewHostPort: Maps container:3000 (preview server) for browser access
 * - sandboxApiHostPort: Maps container:8001 (sandbox API) for health checks and API access from host
 */
export const DockerExecutionTargetSchema = BaseExecutionTargetSchema.extend({
  mode: z.literal('docker'),
  containerName: z.string().optional(), // Not known until container is created
  containerId: z.string().optional(),
  sandboxApiHostPort: z.number().optional(), // Host port mapped to container:8001 (sandbox API)
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
export const KubernetesExecutionTargetSchema = BaseExecutionTargetSchema.extend({
  mode: z.literal('kubernetes'),
  podName: z.string().optional(), // Not known until pod is created
  podNamespace: z.string(),
  podIp: z.string().optional(), // Available once pod is running
  podPhase: KubernetesPodPhaseSchema.optional(), // Raw K8s pod phase for detailed status
  workerPort: z.number().default(8001),
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
 * Uses unified status field for consistent checking across all target types
 */
export function isWorkspaceAvailable(target: ExecutionTarget): boolean {
  return target.status === 'ready'
}

/**
 * Get sandbox API URL from execution target
 * Returns null if the sandbox URL is not available (e.g., pod not running)
 */
export function getSandboxUrl(target: ExecutionTarget): string | null {
  if (target.mode === 'docker') {
    const dockerTarget = target as DockerExecutionTarget
    return dockerTarget.containerName ? `http://${dockerTarget.containerName}:8001` : null
  }
  if (target.mode === 'kubernetes') {
    const k8sTarget = target as KubernetesExecutionTarget
    if (!k8sTarget.podIp) {
      return null
    }
    return `http://${k8sTarget.podIp}:${k8sTarget.workerPort}`
  }
  return null
}
