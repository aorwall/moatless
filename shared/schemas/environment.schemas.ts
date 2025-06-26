import { z } from 'zod'

// Repository schemas
export const LocalRepositorySchema = z.object({
  type: z.literal('local'),
  path: z.string(),
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
})

export const GithubRepositorySchema = z.object({
  type: z.literal('github'),
  url: z.string().optional(),
  repoId: z.string(),
  branch: z.string(),
  commit: z.string().optional(),
  token: z.string().optional(),
})

export const RepositorySchema = z.discriminatedUnion('type', [
  LocalRepositorySchema,
  GithubRepositorySchema,
])

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

// Environment schemas
export const EnvironmentSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string().optional(),
  repository: RepositorySchema,
  codeExecution: CodeExecutionSchema,
  variables: z.record(z.string()),
  createdAt: z.date(),
  updatedAt: z.date(),
})

export const CreateEnvironmentSchema = z.object({
  name: z.string(),
  description: z.string().optional(),
  repository: RepositorySchema,
  codeExecution: CodeExecutionSchema,
  variables: z.record(z.string()).optional(),
})

export const UpdateEnvironmentSchema = z.object({
  name: z.string().optional(),
  description: z.string().optional(),
  repository: RepositorySchema.optional(),
  codeExecution: CodeExecutionSchema.optional(),
  variables: z.record(z.string()).optional(),
})

// Other schemas
export const EnvironmentIdParamsSchema = z.object({
  environmentId: z.string(),
})

export const FileSearchSchema = z.object({
  query: z.string().min(1),
  limit: z.number().min(1).max(1000).optional(),
})

// Type exports
export type Environment = z.infer<typeof EnvironmentSchema>
export type Repository = z.infer<typeof RepositorySchema>
export type LocalRepository = z.infer<typeof LocalRepositorySchema>
export type GithubRepository = z.infer<typeof GithubRepositorySchema>
export type CodeExecution = z.infer<typeof CodeExecutionSchema>
export type CreateEnvironmentRequest = z.infer<typeof CreateEnvironmentSchema>
export type UpdateEnvironmentRequest = z.infer<typeof UpdateEnvironmentSchema>
