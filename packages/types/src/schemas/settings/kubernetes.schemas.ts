import { z } from 'zod'

/**
 * Kubernetes toleration configuration
 */
export const KubernetesTolerationSchema = z.object({
  key: z.string(),
  operator: z.enum(['Exists', 'Equal']),
  value: z.string().optional(),
  effect: z.enum(['NoSchedule', 'PreferNoSchedule', 'NoExecute']),
  tolerationSeconds: z.number().optional(),
})

export type KubernetesToleration = z.infer<typeof KubernetesTolerationSchema>

/**
 * Kubernetes resource limits/requests configuration
 */
export const KubernetesResourcesSchema = z.object({
  cpu: z.string().optional(), // e.g., '500m', '1', '2'
  memory: z.string().optional(), // e.g., '512Mi', '1Gi', '2Gi'
})

export type KubernetesResources = z.infer<typeof KubernetesResourcesSchema>

/**
 * Workspace execution mode
 * - local: Run workspaces directly on the host machine
 * - docker: Run workspaces in Docker containers (legacy)
 * - kubernetes: Run workspaces as Kubernetes pods
 */
export const WorkspaceExecutionModeSchema = z.enum(['local', 'docker', 'kubernetes'])

export type WorkspaceExecutionMode = z.infer<typeof WorkspaceExecutionModeSchema>

/**
 * Kubernetes settings for pod-based workspace execution
 */
export const KubernetesSettingsSchema = z.object({
  /** Whether Kubernetes execution mode is enabled */
  enabled: z.boolean().default(false),

  /** Kubernetes namespace for worker pods */
  namespace: z.string().default('moatless'),

  /** Path to kubeconfig file (undefined means use in-cluster config) */
  kubeconfig: z.string().optional(),

  /** Worker container image */
  image: z.string().default('moatless/worker:latest'),

  /** Image pull secrets for private registries */
  imagePullSecrets: z.array(z.string()).optional(),

  /** Service account for pods */
  serviceAccount: z.string().optional(),

  /** Resource limits for pods */
  resourceLimits: KubernetesResourcesSchema.optional(),

  /** Resource requests for pods */
  resourceRequests: KubernetesResourcesSchema.optional(),

  /** Node selector for pod scheduling */
  nodeSelector: z.record(z.string(), z.string()).optional(),

  /** Tolerations for pod scheduling */
  tolerations: z.array(KubernetesTolerationSchema).optional(),

  /** Pod startup timeout in seconds */
  podStartupTimeoutSeconds: z.number().default(120),

  /** Health check interval in seconds */
  healthCheckIntervalSeconds: z.number().default(30),

  /** Main API URL for pods to connect back (required for event forwarding) */
  workerApiUrl: z.string().optional(),

  /** API key for worker authentication (optional) */
  workerApiKey: z.string().optional(),

  /** Path to worker pod template file (YAML) */
  workerPodTemplate: z.string().optional(),

  // Per-task PVC settings for workspace persistence
  /** Enable per-task PVC for workspace data persistence */
  workerStorageEnabled: z.boolean().default(true),

  /** Storage class for worker PVCs (empty = cluster default) */
  workerStorageClass: z.string().optional(),

  /** Size of each task's PVC (e.g., '10Gi', '20Gi') */
  workerStorageSize: z.string().default('10Gi'),
})

export type KubernetesSettings = z.infer<typeof KubernetesSettingsSchema>

/**
 * Default Kubernetes settings
 */
export const defaultKubernetesSettings: KubernetesSettings = {
  enabled: false,
  namespace: 'moatless',
  image: 'moatless/worker:latest',
  podStartupTimeoutSeconds: 120,
  healthCheckIntervalSeconds: 30,
  workerStorageEnabled: true,
  workerStorageSize: '10Gi',
}
