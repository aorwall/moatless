import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

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

export const ServerConfigSchema = z.object({
  /** Unique server identifier (e.g., "preview", "files", "storybook") */
  name: z.string(),
  /** Human-readable label shown in UI (defaults to name if not provided) */
  label: z.string().optional(),
  /** Port the server listens on inside the sandbox container (assigned at provisioning time) */
  port: z.number().optional(),
  /** Whether this server is built into the sandbox image (not user-configurable) */
  builtin: z.boolean().default(false).optional(),
  /** Commands to start the server (like previewConfig.serverStartCommands) */
  startCommands: z.array(z.string()).optional(),
  /** Startup timeout in seconds */
  startupTimeoutSeconds: z.number().min(5).default(30).optional(),
  /** Server has built-in auto-reload */
  autoReload: z.boolean().default(false).optional(),
  /** Inject inspector script (only relevant for preview-type servers) */
  enableInspector: z.boolean().default(false).optional(),
  /** Whether this server is the default view when opening a task */
  default: z.boolean().default(false).optional(),
  /** Per-server environment variables */
  envVars: z.record(z.string(), z.string()).optional(),
})

export const CodeExecutionSchema = z.object({
  setupCommands: z.array(z.string()).optional(),
  dockerImage: z.string().optional(),
  envVars: z.record(z.string(), z.string()).optional(),
})

/**
 * Agent configuration schema for REPO_CONFIG.agentConfig
 * Contains static agent settings that don't change per-request
 */
export const AgentConfigSchema = z.object({
  /** Plugin identifiers to enable (format: {plugin-name}@{marketplace-name}) */
  plugins: z.array(z.string()).optional(),
  /** Pre-resolved relative paths for plugins (from backend PluginService) */
  pluginPaths: z.array(z.string()).optional(),
  /** Default execution timeout in milliseconds */
  timeout: z.number().optional(),
  /** Default model for agent execution */
  model: z.string().optional(),
})

// Channel information schema - dynamically added based on config and current user
export const ChannelInfoSchema = z.object({
  email: z
    .object({
      address: z.string().describe('Email address for creating tasks in this repository'),
    })
    .optional(),
})

// Schema for .moatless/config.json file in repositories
// Also used for REPO_CONFIG environment variable in sandbox
export const RepositoryConfigFileSchema = z.object({
  systemPrompt: z
    .string()
    .nullish()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
  servers: z
    .array(ServerConfigSchema)
    .optional()
    .describe('Server configurations for this repository'),
  agentConfig: AgentConfigSchema.optional().describe(
    'Static agent configuration (plugins, timeout, model) that applies to all requests',
  ),
})

// Shared base schema for all repository types
const BaseRepositorySchema = z.object({
  id: z.string(),
  name: repositoryNameSchema,
  description: z.string().optional(),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema,
  previewConfig: PreviewConfigSchema.optional(),
  servers: z
    .array(ServerConfigSchema)
    .optional()
    .describe('Server configurations for this repository'),
  deleted: z.boolean().optional().describe('Soft deletion flag'),
  createdBy: z.string().optional().describe('User ID who created this repository'),
  visibility: z
    .enum(['private', 'public'])
    .default('private')
    .describe("Repository visibility: 'private' = creator only, 'public' = all users can read"),
  createdAt: dateSchema,
  updatedAt: dateSchema,
  // Channel information - dynamically added based on config and current user
  channels: ChannelInfoSchema.optional(),
  // Plugin identifiers to enable for this repository
  plugins: z
    .array(z.string())
    .optional()
    .describe('Plugin identifiers to enable (format: {plugin-name}@{marketplace-name})'),
})

// Remote repository schema - for remote repositories (no persistent clone)
export const RemoteRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('remote'),
  remote: z.object({
    url: z.url('Must be a valid Git URL').describe('Repository URL'),
    provider: z.enum(['github', 'gitlab']).describe('Git provider type'),
    defaultBranch: z.string().optional().describe('Default branch name from provider'),
  }),
})

// Template repository schema - for source template repositories
// These are reusable templates that can be cloned to create new remote repositories
// Template repositories are NOT directly used for tasks - they are only used to create repos
export const TemplateRepositorySchema = BaseRepositorySchema.extend({
  kind: z.literal('template'),
  remote: z.object({
    url: z.url('Must be a valid Git URL').describe('Git repository URL for the template'),
    provider: z.enum(['github', 'gitlab']).describe('Git provider type'),
    defaultBranch: z.string().optional().describe('Default branch name from provider'),
  }),
})

// Discriminated union of all repository types
export const RepositoryConfigSchema = z.discriminatedUnion('kind', [
  RemoteRepositorySchema,
  TemplateRepositorySchema,
])

// Creation schemas (separate for each type)
export const CreateRemoteRepositorySchema = z.object({
  kind: z.literal('remote'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  remoteUrl: z.url('Must be a valid Git URL').describe('Repository URL'),
  provider: z.enum(['github', 'gitlab']).optional(),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
  servers: z.array(ServerConfigSchema).optional(),
  visibility: z
    .enum(['private', 'public'])
    .default('private')
    .optional()
    .describe("Repository visibility: 'private' = creator only, 'public' = all users can read"),
})

// Schema for creating a new template repository (source template)
export const CreateTemplateRepositorySchema = z.object({
  kind: z.literal('template'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  remoteUrl: z.url('Must be a valid Git URL').describe('Git repository URL for the template'),
  provider: z.enum(['github', 'gitlab']).optional(),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
  servers: z.array(ServerConfigSchema).optional(),
  visibility: z
    .enum(['private', 'public'])
    .default('private')
    .optional()
    .describe("Repository visibility: 'private' = creator only, 'public' = all users can read"),
})

// Schema for creating a remote repository FROM a template
// This creates a new GitHub repository by cloning from the template
export const CreateRemoteFromTemplateSchema = z.object({
  kind: z.literal('remote'),
  name: repositoryNameSchema,
  description: z.string().optional(),
  templateId: z.string().describe('Template repository ID to create from'),
  systemPrompt: z
    .string()
    .optional()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
  servers: z.array(ServerConfigSchema).optional(),
  visibility: z
    .enum(['private', 'public'])
    .default('private')
    .optional()
    .describe("Repository visibility: 'private' = creator only, 'public' = all users can read"),
})

// Union of creation requests
export const CreateRepositorySchema = z.discriminatedUnion('kind', [
  CreateRemoteRepositorySchema,
  CreateTemplateRepositorySchema,
])

export const UpdateRepositorySchema = z.object({
  name: repositoryNameSchema.optional(),
  description: z.string().nullish(),
  // Remote/Template repository fields
  remoteUrl: z.url().nullish(),
  provider: z.enum(['github', 'gitlab']).optional(),
  defaultBranch: z.string().nullish(),
  // Shared fields
  systemPrompt: z
    .string()
    .nullish()
    .describe('Additional system prompt to append to Claude Code default prompt'),
  codeExecution: CodeExecutionSchema.optional(),
  previewConfig: PreviewConfigSchema.optional(),
  servers: z.array(ServerConfigSchema).optional(),
  visibility: z
    .enum(['private', 'public'])
    .optional()
    .describe("Repository visibility: 'private' = creator only, 'public' = all users can read"),
  // Plugin identifiers to enable
  plugins: z
    .array(z.string())
    .optional()
    .describe('Plugin identifiers to enable (format: {plugin-name}@{marketplace-name})'),
})

export const ConvertToTemplateSchema = z.object({
  repositoryId: z.string(),
})

export const RepositoryListResponseSchema = z.object({
  repositories: z.array(RepositoryConfigSchema),
})

export const RepositoryFileSearchResultSchema = z.object({
  path: z.string(),
  type: z.string(),
  size: z.number().optional(),
})

z.object({
  files: z.array(RepositoryFileSearchResultSchema),
})

export const BranchListResponseSchema = z.object({
  branches: z.array(z.string()),
})

z.object({
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
export type RemoteRepository = z.infer<typeof RemoteRepositorySchema>
export type TemplateRepository = z.infer<typeof TemplateRepositorySchema>
export type CodeExecution = z.infer<typeof CodeExecutionSchema>
export type PreviewConfig = z.infer<typeof PreviewConfigSchema>
export type ServerConfig = z.infer<typeof ServerConfigSchema>
export type ChannelInfo = z.infer<typeof ChannelInfoSchema>
export type RepositoryConfigFile = z.infer<typeof RepositoryConfigFileSchema>
export type AgentConfig = z.infer<typeof AgentConfigSchema>
export type CreateRepositoryRequest = z.infer<typeof CreateRepositorySchema>
export type CreateRemoteRepositoryRequest = z.infer<typeof CreateRemoteRepositorySchema>
export type CreateTemplateRepositoryRequest = z.infer<typeof CreateTemplateRepositorySchema>
export type CreateRemoteFromTemplateRequest = z.infer<typeof CreateRemoteFromTemplateSchema>
export type UpdateRepositoryRequest = z.infer<typeof UpdateRepositorySchema>
export type ConvertToTemplateRequest = z.infer<typeof ConvertToTemplateSchema>
export type BranchListResponse = z.infer<typeof BranchListResponseSchema>
