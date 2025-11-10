import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { WorkspaceStatusResponseSchema } from './workspace.schemas.js'

const repositoryNameSchema = z
  .string()
  .min(1, 'Repository name is required')
  .max(255, 'Repository name must be less than 255 characters')

export const PreviewConfigSchema = z.object({
  serverStartCommands: z
    .array(z.string())
    .min(1, 'At least one server start command is required')
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
  autoReload: z
    .boolean()
    .default(false)
    .optional()
    .describe(
      'Whether the server has built-in auto-reload/watch functionality. If true, the server will automatically reload when files change and manual restarts are unnecessary (default: false)',
    ),
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

// Remote repository schema - for remote repositories (no persistent clone)
export const RemoteRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('remote'),
  remote: z.object({
    url: z.string().url('Must be a valid Git URL').describe('Repository URL'),
    provider: z.enum(['github', 'gitlab', 'other']).describe('Git provider type'),
    defaultBranch: z.string().optional().describe('Default branch name from provider'),
  }),
})

// Template repository schema - for source template repositories
// These are reusable templates that can be cloned to create new local repositories
// Template repositories are NOT directly used for tasks - they are only used to create local repos
// Similar to remote repositories but with additional configuration for creating local repos
export const TemplateRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('template'),
  remote: z.object({
    url: z.string().url('Must be a valid Git URL').describe('Git repository URL for the template'),
    provider: z.enum(['github', 'gitlab', 'other']).default('github').describe('Git provider type'),
    defaultBranch: z.string().optional().describe('Default branch name from provider'),
  }),
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
  remoteUrl: z.string().url('Must be a valid Git URL').describe('Repository URL'),
  provider: z.enum(['github', 'gitlab', 'other']).default('github').optional(),
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

// Schema for creating a new template repository (source template)
export const CreateTemplateRepositorySchema = z.object({
  kind: z.literal('template'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  remoteUrl: z
    .string()
    .url('Must be a valid Git URL')
    .describe('Git repository URL for the template'),
  provider: z.enum(['github', 'gitlab', 'other']).default('github').optional(),
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

// Schema for creating a local repository FROM a template
export const CreateLocalFromTemplateSchema = z.object({
  kind: z.literal('local'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  templateId: z.string().describe('Template repository ID to clone from'),
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
  // Local repository fields
  path: z.string().optional(),
  useGitWorktrees: z.boolean().optional(),
  // Remote/Template repository fields (both use remoteUrl)
  remoteUrl: z.string().url().nullish(),
  provider: z.enum(['github', 'gitlab', 'other']).optional(),
  defaultBranch: z.string().nullish(),
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
export type CreateLocalFromTemplateRequest = z.infer<typeof CreateLocalFromTemplateSchema>
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
