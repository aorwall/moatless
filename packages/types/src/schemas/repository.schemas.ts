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
  setupCommands: z.array(z.string()).optional(),
  dockerImage: z.string().optional(),
  envVars: z.record(z.string(), z.string()).optional(),
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

// GitHub URL validation
const githubUrlSchema = z
  .string()
  .regex(
    /^https:\/\/github\.com\/[^/]+\/[^/.]+(?:\.git)?$/,
    'Must be a valid GitHub HTTPS URL (e.g., https://github.com/owner/repo)',
  )

// Shared base schema for all repository types
const BaseRepositorySchema = z.object({
  id: z.string(),
  name: repositoryNameSchema,
  description: z.string().optional(),
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
  deleted: z.boolean().optional().describe('Soft deletion flag'),
  createdAt: dateSchema,
  updatedAt: dateSchema,
})

// Local repository schema - for user-managed directories
export const LocalRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('local'),
  path: z.string().describe('Local filesystem path to the repository'),
  useGitWorktrees: z.boolean().default(false),
  originalBranch: z.string().optional(),
})

// Remote repository schema - for GitHub repositories (no persistent clone)
export const RemoteRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('remote'),
  remote: z.object({
    url: githubUrlSchema.describe('GitHub repository URL'),
    provider: z.literal('github').describe('Git provider type'),
    defaultBranch: z.string().optional().describe('Default branch name from provider'),
  }),
})

// Template repository schema - for repositories created from templates
export const TemplateRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('template'),
  templateId: z.string().describe('Template identifier'),
  path: z.string().describe('Path where template was cloned'),
  useGitWorktrees: z.boolean().default(false),
  originalBranch: z.string().optional(),
})

// Discriminated union of all repository types
export const RepositoryConfigSchema = z.discriminatedUnion('kind', [
  LocalRepositorySchema,
  RemoteRepositorySchema,
  TemplateRepositorySchema,
])

// Creation schemas (separate for each type)
export const CreateLocalRepositorySchema = z.object({
  kind: z.literal('local'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  path: z.string().describe('Local filesystem path to the repository'),
  useGitWorktrees: z.boolean().default(false).optional(),
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

export const CreateRemoteRepositorySchema = z.object({
  kind: z.literal('remote'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  remoteUrl: githubUrlSchema.describe('GitHub repository URL'),
  provider: z.literal('github').default('github').optional(),
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

export const CreateTemplateRepositorySchema = z.object({
  kind: z.literal('template'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  templateId: z.string().describe('Template identifier'),
  // Note: path is NOT provided by user, generated during creation
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

// Union of creation requests
export const CreateRepositorySchema = z.discriminatedUnion('kind', [
  CreateLocalRepositorySchema,
  CreateRemoteRepositorySchema,
  CreateTemplateRepositorySchema,
])

export const UpdateRepositorySchema = z.object({
  name: repositoryNameSchema.optional(),
  description: z.string().nullish(),
  // Local/Template repository fields
  path: z.string().optional(),
  useGitWorktrees: z.boolean().optional(),
  // Remote repository fields
  remoteUrl: githubUrlSchema.nullish(),
  provider: z.literal('github').optional(),
  // Shared fields
  originalBranch: z.string().nullish(),
  syncWithRepoConfig: z
    .boolean()
    .optional()
    .describe('Whether to sync configuration from .moatless/config.json in the repository'),
  systemPrompt: z
    .string()
    .nullish()
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
export type LocalRepository = z.infer<typeof LocalRepositorySchema>
export type RemoteRepository = z.infer<typeof RemoteRepositorySchema>
export type TemplateRepository = z.infer<typeof TemplateRepositorySchema>
export type CodeExecution = z.infer<typeof CodeExecutionSchema>
export type PreviewConfig = z.infer<typeof PreviewConfigSchema>
export type RepositoryConfigFile = z.infer<typeof RepositoryConfigFileSchema>
export type CreateRepositoryRequest = z.infer<typeof CreateRepositorySchema>
export type CreateLocalRepositoryRequest = z.infer<typeof CreateLocalRepositorySchema>
export type CreateRemoteRepositoryRequest = z.infer<typeof CreateRemoteRepositorySchema>
export type CreateTemplateRepositoryRequest = z.infer<typeof CreateTemplateRepositorySchema>
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
