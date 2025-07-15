import { z } from 'zod'

// Repository name validation - allows URL-safe names or GitHub-style owner/repo format
const repositoryNameSchema = z
  .string()
  .min(1, 'Repository name is required')
  .max(255, 'Repository name must be less than 255 characters')
  .regex(
    /^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$|^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?\/[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$/,
    'Repository name must be URL-safe (letters, numbers, dots, hyphens, underscores) or in GitHub format (owner/repo). Cannot start or end with special characters.',
  )

// Script injection strategy schema
export const ScriptInjectionStrategySchema = z.object({
  method: z.enum(['html-injection', 'middleware', 'build-plugin', 'proxy', 'none']),
  config: z.record(z.any()).optional(),
})

// Preview repository configuration
export const PreviewConfigSchema = z.object({
  framework: z.enum(['vite', 'nextjs', 'cra', 'webpack', 'unknown']).optional(),
  frameworkVersion: z.string().optional(),
  serverStartCommand: z.string().optional(),
  injectionStrategy: ScriptInjectionStrategySchema.optional(),
})

// Code execution schema
export const CodeExecutionSchema = z.object({
  type: z.enum(['local', 'docker']),
  setupCommands: z.array(z.string()).optional(),
  // Docker-specific fields
  containerName: z.string().optional(),
  dockerImage: z.string().optional(),
  envVars: z.record(z.string()).optional(),
  volumes: z.array(z.string()).optional(),
  ports: z.record(z.number()).optional(),
  volumeMapping: z
    .object({
      hostPath: z.string(),
      containerPath: z.string(),
    })
    .optional(),
  resources: z
    .object({
      cpuLimit: z.string(),
      memoryLimit: z.string(),
    })
    .optional(),
})

// Repository schemas - merged repository fields directly into RepositoryConfig
export const RepositoryConfigSchema = z.object({
  id: z.string(),
  name: repositoryNameSchema,
  description: z.string().optional(),
  // Repository fields moved here
  path: z.string(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),

  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  createdAt: z.date(),
  updatedAt: z.date(),
})

export const CreateRepositorySchema = z.object({
  name: repositoryNameSchema,
  description: z.string().optional(),
  // Repository fields
  path: z.string(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  // Original fields
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
})

export const UpdateRepositorySchema = z.object({
  name: repositoryNameSchema.optional(),
  description: z.string().optional(),
  // Repository fields
  path: z.string().optional(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  // Original fields
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
})

// Other schemas
export const RepositoryIdParamsSchema = z.object({
  repositoryId: z.string(),
})

export const FileSearchSchema = z.object({
  query: z.string().min(1),
  limit: z.number().min(1).max(1000).optional(),
})

// Type exports
export type RepositoryConfig = z.infer<typeof RepositoryConfigSchema>
export type CodeExecution = z.infer<typeof CodeExecutionSchema>
export type PreviewConfig = z.infer<typeof PreviewConfigSchema>
export type ScriptInjectionStrategy = z.infer<typeof ScriptInjectionStrategySchema>
export type CreateRepositoryRequest = z.infer<typeof CreateRepositorySchema>
export type UpdateRepositoryRequest = z.infer<typeof UpdateRepositorySchema>
