import { z } from 'zod'

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
  serverPort: z.number().optional(),
  injectionStrategy: ScriptInjectionStrategySchema.optional(),
  autoDetected: z.boolean().optional(),
})

// Code execution schema
export const CodeExecutionSchema = z.object({
  type: z.enum(['local', 'docker']),
  setupCommands: z.string().optional(),
  serverCommand: z.string().optional(),
  serverPortPlaceholder: z.string().optional(),
  serverLogFile: z.string().optional(),
  image: z.string().optional(),
  envVars: z.record(z.string()).optional(),
  volumes: z.array(z.string()).optional(),
  ports: z.record(z.number()).optional(),
})

// Repository schemas - merged repository fields directly into RepositoryConfig
export const RepositoryConfigSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().optional(),
  // Repository fields moved here
  path: z.string(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  // Original fields
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  variables: z.record(z.string()),
  createdAt: z.date(),
  updatedAt: z.date(),
})

export const CreateRepositorySchema = z.object({
  name: z.string(),
  description: z.string().optional(),
  // Repository fields
  path: z.string(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  // Original fields
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  variables: z.record(z.string()).optional(),
})

export const UpdateRepositorySchema = z.object({
  name: z.string().optional(),
  description: z.string().optional(),
  // Repository fields
  path: z.string().optional(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  // Original fields
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
  variables: z.record(z.string()).optional(),
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
