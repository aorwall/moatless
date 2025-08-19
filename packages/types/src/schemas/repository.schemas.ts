import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { WorkspaceStatusResponseSchema } from './workspace.schemas.js'

// Repository name validation - allows URL-safe names or GitHub-style owner/repo format
const repositoryNameSchema = z
  .string()
  .min(1, 'Repository name is required')
  .max(255, 'Repository name must be less than 255 characters')
  .regex(
    /^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$|^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?\/[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$/,
    'Repository name must be URL-safe (letters, numbers, dots, hyphens, underscores) or in GitHub format (owner/repo). Cannot start or end with special characters.',
  )

// Preview repository configuration
export const PreviewConfigSchema = z.object({
  serverStartCommands: z.array(z.string()).optional(), // Commands to start the preview server
  takeScreenshots: z.boolean().optional(),
  buildCommand: z.string().optional(),
  buildOutputDirectory: z.string().default('dist').optional(),
  enableInspector: z.boolean().default(true).optional(),
})

// Code execution schema
export const CodeExecutionSchema = z.object({
  type: z.enum(['local', 'docker']),
  setupCommands: z.array(z.string()).optional(),
  // Docker-specific fields
  dockerImage: z.string().optional(),
  envVars: z.record(z.string(), z.string()).optional(),
  volumes: z.array(z.string()).optional(),
  ports: z.record(z.string(), z.number()).optional(),
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
  remoteUrl: z.string().optional(), // GitHub or other remote URL
  sourceType: z.enum(['local', 'github', 'template']).optional(), // Source of the repository
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  deleted: z.boolean().optional(), // Soft deletion flag
  templateId: z.string().optional(), // Template ID when created from template
  vibe: z.boolean().optional(), // Flag to identify repositories created from templates

  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  createdAt: dateSchema,
  updatedAt: dateSchema,
})

export const CreateRepositorySchema = z.object({
  name: repositoryNameSchema,
  description: z.string().optional(),
  // Repository fields
  path: z.string().optional(), // Made optional - will be auto-generated for GitHub repos
  templatePath: z.string().optional(), // Path to template directory to copy from
  remoteUrl: z.string().optional(), // GitHub URL for cloning
  sourceType: z.enum(['local', 'github', 'template']).optional(), // Source of the repository
  useGitWorktrees: z.boolean().optional(),
  originalBranch: z.string().optional(),
  templateId: z.string().optional(), // Template ID when created from template
  vibe: z.boolean().optional(), // Flag to identify repositories created from templates
  // Original fields
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
})

export const UpdateRepositorySchema = z.object({
  name: repositoryNameSchema.optional(),
  description: z.string().optional(),
  // Repository fields
  path: z.string().optional(),
  remoteUrl: z.string().optional(),
  sourceType: z.enum(['local', 'github']).optional(),
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

// Repository verification returns workspace status
export const VerificationResponseSchema = WorkspaceStatusResponseSchema

// Suggested repository schema
export const SuggestedRepositorySchema = z.object({
  name: z.string(),
  path: z.string(),
})

export const SuggestedRepositoriesResponseSchema = z.object({
  suggestions: z.array(SuggestedRepositorySchema),
})

// Repository list response schema
export const RepositoryListResponseSchema = z.object({
  repositories: z.array(RepositoryConfigSchema),
})

// File search response schemas
export const RepositoryFileSearchResultSchema = z.object({
  path: z.string(),
  type: z.string(),
  size: z.number().optional(),
})

export const RepositoryFileSearchResponseSchema = z.object({
  files: z.array(RepositoryFileSearchResultSchema),
})

// Branch list response schema
export const BranchListResponseSchema = z.object({
  branches: z.array(z.string()),
})

// Preview config response schema
export const PreviewConfigResponseSchema = z.object({
  previewConfig: z.object({
    framework: z.string().optional(),
    buildCommand: z.string().optional(),
    devCommand: z.string().optional(),
    startCommand: z.string().optional(),
    installCommand: z.string().optional(),
    port: z.number().optional(),
  }),
})

// Type exports
export type RepositoryConfig = z.infer<typeof RepositoryConfigSchema>
export type CodeExecution = z.infer<typeof CodeExecutionSchema>
export type PreviewConfig = z.infer<typeof PreviewConfigSchema>
export type CreateRepositoryRequest = z.infer<typeof CreateRepositorySchema>
export type UpdateRepositoryRequest = z.infer<typeof UpdateRepositorySchema>
export type VerificationResponse = z.infer<typeof VerificationResponseSchema>
export type SuggestedRepository = z.infer<typeof SuggestedRepositorySchema>
export type SuggestedRepositoriesResponse = z.infer<typeof SuggestedRepositoriesResponseSchema>
export type RepositoryListResponse = z.infer<typeof RepositoryListResponseSchema>
export type RepositoryFileSearchResult = z.infer<typeof RepositoryFileSearchResultSchema>
export type RepositoryFileSearchResponse = z.infer<typeof RepositoryFileSearchResponseSchema>
export type BranchListResponse = z.infer<typeof BranchListResponseSchema>
export type PreviewConfigResponse = z.infer<typeof PreviewConfigResponseSchema>
