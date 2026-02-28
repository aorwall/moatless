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
 * Sandbox execution mode
 * - docker: Run sandboxes in Docker containers (local development)
 * - kubernetes: Run sandboxes as Kubernetes pods (production)
 */
export const SandboxExecutionModeSchema = z.enum(['docker', 'kubernetes'])

export type SandboxExecutionMode = z.infer<typeof SandboxExecutionModeSchema>

/**
 * Kubernetes settings for pod-based workspace execution
 */
export const KubernetesSettingsSchema = z.object({
  /** Whether Kubernetes execution mode is enabled */
  enabled: z.boolean().default(false),

  /** Kubernetes namespace for sandbox pods */
  namespace: z.string().default('moatless'),

  /** Path to kubeconfig file (undefined means use in-cluster config) */
  kubeconfig: z.string().optional(),

  /** Sandbox container image */
  image: z.string().default('aorwall/moatless-vibe-sandbox-js:latest'),

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
})

export type KubernetesSettings = z.infer<typeof KubernetesSettingsSchema>

/**
 * Default Kubernetes settings
 */
export const defaultKubernetesSettings: KubernetesSettings = {
  enabled: false,
  namespace: 'moatless',
  image: 'aorwall/moatless-vibe-sandbox',
  podStartupTimeoutSeconds: 120,
  healthCheckIntervalSeconds: 30,
}
