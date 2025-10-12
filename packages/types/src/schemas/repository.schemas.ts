import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { WorkspaceStatusResponseSchema } from './workspace.schemas.js'

const repositoryNameSchema = z
  .string()
  .min(1, 'Repository name is required')
  .max(255, 'Repository name must be less than 255 characters')
  .regex(
    /^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$|^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?\/[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])?$/,
    'Repository name must be URL-safe (letters, numbers, dots, hyphens, underscores) or in GitHub format (owner/repo). Cannot start or end with special characters.',
  )

export const PreviewConfigSchema = z.object({
  serverStartCommands: z
    .array(z.string())
    .optional()
    .describe('Commands to start the preview server'),
  takeScreenshots: z.boolean().optional(),
  buildCommand: z.string().optional(),
  buildOutputDirectory: z.string().default('dist').optional(),
  enableInspector: z.boolean().default(true).optional(),
  startupTimeoutSeconds: z
    .number()
    .min(5)
    .default(30)
    .optional()
    .describe('Time to wait for server to become HTTP-responsive (default: 30 seconds)'),
})

export const CodeExecutionSchema = z.object({
  type: z.enum(['local', 'docker']),
  setupCommands: z.array(z.string()).optional(),
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

// Schema for .moatless/config.json file in repositories
export const RepositoryConfigFileSchema = z.object({
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
})

// Remote URL validation - only accept GitHub HTTPS URLs
const remoteUrlSchema = z
  .string()
  .regex(
    /^https:\/\/github\.com\/[^/]+\/[^/.]+(?:\.git)?$/,
    'Remote URL must be a valid GitHub HTTPS URL (e.g., https://github.com/owner/repo)',
  )
  .optional()

export const RepositoryConfigSchema = z.object({
  id: z.string(),
  name: repositoryNameSchema,
  description: z.string().optional(),
  path: z.string(),
  remoteUrl: remoteUrlSchema,
  sourceType: z
    .enum(['local', 'github', 'template'])
    .optional()
    .describe('Source of the repository'),
  useGitWorktrees: z.boolean().optional(),
  cloneOnNewTask: z
    .boolean()
    .default(false)
    .describe('Whether to clone repository for each new task instead of using worktrees'),
  originalBranch: z.string().optional(),
  deleted: z.boolean().optional().describe('Soft deletion flag'),
  templateId: z.string().optional().describe('Template ID when created from template'),
  vibe: z.boolean().optional().describe('Flag to identify repositories created from templates'),
  syncWithRepoConfig: z
    .boolean()
    .optional()
    .describe('Whether to sync configuration from .moatless/config.json in the repository'),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  createdAt: dateSchema,
  updatedAt: dateSchema,
})

export const CreateRepositorySchema = z.object({
  name: repositoryNameSchema,
  description: z.string().optional(),
  path: z.string().optional().describe('Made optional - will be auto-generated for GitHub repos'),
  templatePath: z.string().optional().describe('Path to template directory to copy from'),
  remoteUrl: remoteUrlSchema,
  sourceType: z
    .enum(['local', 'github', 'template'])
    .optional()
    .describe('Source of the repository'),
  useGitWorktrees: z.boolean().optional(),
  cloneOnNewTask: z
    .boolean()
    .default(false)
    .describe('Whether to clone repository for each new task instead of using worktrees'),
  originalBranch: z.string().optional(),
  vibe: z.boolean().optional().describe('Flag to identify repositories created from templates'),
  syncWithRepoConfig: z
    .boolean()
    .optional()
    .describe('Whether to sync configuration from .moatless/config.json in the repository'),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
})

export const UpdateRepositorySchema = z.object({
  name: repositoryNameSchema.optional(),
  description: z.string().optional(),
  path: z.string().optional(),
  remoteUrl: remoteUrlSchema,
  sourceType: z.enum(['local', 'github']).optional(),
  useGitWorktrees: z.boolean().optional(),
  cloneOnNewTask: z
    .boolean()
    .optional()
    .describe('Whether to clone repository for each new task instead of using worktrees'),
  originalBranch: z.string().optional(),
  syncWithRepoConfig: z
    .boolean()
    .optional()
    .describe('Whether to sync configuration from .moatless/config.json in the repository'),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
})

export const RepositoryIdParamsSchema = z.object({
  repositoryId: z.string(),
})

export const RepositoryFileSearchSchema = z.object({
  query: z.string().min(1),
  limit: z.number().min(1).max(1000).optional(),
})

export const VerificationResponseSchema = WorkspaceStatusResponseSchema

export const SuggestedRepositorySchema = z.object({
  name: z.string(),
  path: z.string(),
})

export const SuggestedRepositoriesResponseSchema = z.object({
  suggestions: z.array(SuggestedRepositorySchema),
})

export const RepositoryListResponseSchema = z.object({
  repositories: z.array(RepositoryConfigSchema),
})

export const RepositoryFileSearchResultSchema = z.object({
  path: z.string(),
  type: z.string(),
  size: z.number().optional(),
})

export const RepositoryFileSearchResponseSchema = z.object({
  files: z.array(RepositoryFileSearchResultSchema),
})

export const BranchListResponseSchema = z.object({
  branches: z.array(z.string()),
})

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
export type RepositoryConfigFile = z.infer<typeof RepositoryConfigFileSchema>
export type CreateRepositoryRequest = z.infer<typeof CreateRepositorySchema>
export type UpdateRepositoryRequest = z.infer<typeof UpdateRepositorySchema>
export type VerificationResponse = z.infer<typeof VerificationResponseSchema>
export type SuggestedRepository = z.infer<typeof SuggestedRepositorySchema>
export type SuggestedRepositoriesResponse = z.infer<typeof SuggestedRepositoriesResponseSchema>
export type RepositoryListResponse = z.infer<typeof RepositoryListResponseSchema>
export type RepositoryFileSearch = z.infer<typeof RepositoryFileSearchSchema>
export type RepositoryFileSearchResult = z.infer<typeof RepositoryFileSearchResultSchema>
export type RepositoryFileSearchResponse = z.infer<typeof RepositoryFileSearchResponseSchema>
export type BranchListResponse = z.infer<typeof BranchListResponseSchema>
export type PreviewConfigResponse = z.infer<typeof PreviewConfigResponseSchema>
