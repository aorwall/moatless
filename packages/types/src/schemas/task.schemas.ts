import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { TaskStatusSchema } from './base.schemas.js'

// Query schemas

export const FileQuerySchema = z.object({
  path: z.string().optional().default('/'),
  recursive: z.coerce.boolean().optional().default(false),
})

export const TaskFileSearchSchema = z.object({
  query: z.string(),
  limit: z.number().optional(),
})

export const CommandExecutionSchema = z.object({
  command: z.string(),
  timeout: z.number().optional(),
})

export const ServerLogsQuerySchema = z.object({
  lines: z.coerce.number().optional(),
})

// Content schemas
export const FileContentSchema = z.object({
  content: z.string(),
})

// Command schema for task commands endpoint
export const TaskCommandSchema = z.object({
  commandName: z.string(),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional(),
})

// Message schema for task messages endpoint
export const TaskMessageSchema = z.object({
  message: z.string(),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional(),
  agentMode: z.enum(['edit', 'plan']).optional().default('edit'),
})

// Response schemas for OpenAPI
export const TaskIterationSchema = z.object({
  iterationNumber: z.number(),
  startTime: dateSchema,
  endTime: dateSchema.optional(),
  status: z.enum(['created', 'active', 'completed', 'failed']),
  messageCount: z.number(),
  costUsd: z.number(),
  contextTokens: z.number().optional(),
  initialMessage: z.string().optional(),
  commandName: z.string().optional(),
  result: z.string().optional(),
  gitDiff: z.string().optional(),
  commitHash: z.string().optional(),
  merged: z.boolean().optional(),
  agentSessionId: z.string().optional(),
  model: z.string().optional(),
  agentMode: z.enum(['edit', 'plan']).optional(),
  gitStatus: z
    .object({
      status: z.string(),
      commitsAhead: z.number(),
      commitsBehind: z.number(),
      hasUncommittedChanges: z.boolean(),
      currentBranch: z.string(),
      baseBranch: z.string(),
      originExists: z.boolean(),
      hasConflicts: z.boolean(),
    })
    .optional(),
  gitChanges: z
    .object({
      currentBranch: z.string(),
      status: z.string(),
      additions: z.number(),
      deletions: z.number(),
      modifications: z.number(),
    })
    .optional(),
  gitSyncStatus: z
    .object({
      status: z.enum(['synced', 'ahead', 'behind', 'diverged', 'uncommitted_changes', 'error']),
      commitsAhead: z.number(),
      commitsBehind: z.number(),
      hasUncommittedChanges: z.boolean(),
      currentBranch: z.string(),
      baseBranch: z.string(),
      originExists: z.boolean(),
    })
    .optional(),
  attachments: z
    .array(
      z.object({
        fileId: z.string().optional(),
        filename: z.string(),
        path: z.string(),
        mimeType: z.string(),
      }),
    )
    .optional(),
  willNotResume: z.boolean().optional(),
  screenshotPath: z.string().optional(), // Path to preview screenshot
  thumbnailPath: z.string().optional(), // Path to preview screenshot thumbnail (340px width)
  siteUrl: z.string().optional(), // URL of the deployed iteration (e.g., Netlify deployment URL)
  checkpointCommitHash: z.string().optional(), // Git commit hash created for this iteration checkpoint
  checkpointPushed: z.boolean().optional(), // Whether the checkpoint was pushed to remote
})

// ============================================
// Task Response Schema using shared workspace status schemas
// ============================================

export const TaskResponseSchema = z.object({
  taskId: z.string(),
  createdAt: dateSchema,
  lastMessageAt: dateSchema,
  agentType: z.string(),
  repositoryId: z.string(),
  status: TaskStatusSchema,
  numTurns: z.number(),
  totalCostUsd: z.number(),
  iterations: z.array(TaskIterationSchema),
  workspacePath: z.string().optional(),
  templateId: z.string().optional(),
  vibe: z.boolean().optional(), // Flag to identify tasks created from templates
  name: z.string().optional(), // URL-friendly task name (alphanumeric and dashes only)
  description: z.string().optional(), // One-sentence summary of the task
  siteId: z.string().optional(), // Deployment site ID (e.g., Netlify site ID)
  siteProvider: z.string().optional(), // Deployment provider (e.g., "netlify")
  gitCheckpointEnabled: z.boolean().optional(), // Whether git checkpoints are enabled for this task
  gitPushEnabled: z.boolean().optional(), // Whether pushing checkpoints to remote is enabled
  title: z.string().optional(), // Derived title for display (computed from name or first message)
  repositoryName: z.string().optional(), // Repository name for display (computed from repositoryId)
})

export const TaskListResponseSchema = z.object({
  items: z.array(TaskResponseSchema),
  pagination: z.object({
    limit: z.number(),
    offset: z.number(),
    total: z.number().optional(),
    hasMore: z.boolean().optional(),
  }),
})

export const CloseTaskResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
})

export const FileListResponseSchema = z.object({
  files: z.array(
    z.object({
      name: z.string(),
      path: z.string(),
      type: z.enum(['file', 'directory']),
      size: z.number().optional(),
    }),
  ),
  totalCount: z.number(),
})

export const FileContentResponseSchema = z.object({
  content: z.string(),
  encoding: z.string(),
  oldContent: z.string().optional(),
  additions: z.number().optional(),
  deletions: z.number().optional(),
  isModified: z.boolean().optional(),
})

export const FileWriteResponseSchema = z.object({
  success: z.boolean(),
})

export const FileSearchResponseSchema = z.object({
  files: z.array(
    z.object({
      path: z.string(),
      matches: z.number(),
    }),
  ),
  totalMatches: z.number(),
})

export const GitDiffFileSchema = z.object({
  oldFile: z.string().optional(),
  newFile: z.string().optional(),
  hunks: z.array(z.string()),
})

export const GitDiffResponseSchema = z.object({
  diff: z.string().optional(),
  files: z.array(GitDiffFileSchema),
})

// Server status enum for better state management
export const ServerStatusEnumSchema = z.enum([
  'stopped',
  'installing', // Dependencies/setup commands are running
  'starting', // Server process is starting
  'started', // Server is running
  'failed',
])

export const ServerStatusSchema = z.object({
  status: ServerStatusEnumSchema,
  url: z.string().optional(),
  proxyUrl: z.string().optional(),
  pid: z.number().optional(),
  uptimeSeconds: z.number().optional(),
  error: z.string().optional(),
  logs: z.string().optional(),
  currentPath: z.string().optional(), // Current navigation path in preview
})

export const ServerStopResponseSchema = z.object({
  stopped: z.boolean(),
  runtime: z.number(),
})

export const ServerLogsResponseExtendedSchema = z.object({
  logs: z.string(),
  lines: z.number(),
  truncated: z.boolean(),
})

// Request schemas moved from api.schemas.ts

// Create Task
export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  repositoryId: z.string(),
  message: z.string().optional(),
  branchName: z.string().optional(), // Name for the new workspace branch (deprecated, use originalBranch)
  originalBranch: z.string().optional(), // Branch to create from (the source branch)
  newBranch: z.string().optional(), // When set, create branch with this name and also worktree if we use worktree with a filepath normalized name
  start: z.boolean().optional(),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional(), // Absolute file paths for uploaded files
  agentMode: z.enum(['edit', 'plan']).optional(), // Agent mode for task creation
  templateId: z.string().optional(), // Template ID when created from template
  name: z.string().optional(), // URL-friendly task name (alphanumeric and dashes only)
  description: z.string().optional(), // One-sentence summary of the task
  gitCheckpointEnabled: z.boolean().optional(), // Enable automatic git checkpoints after each iteration
  gitPushEnabled: z.boolean().optional(), // Enable pushing checkpoints to remote (requires gitCheckpointEnabled and remoteUrl)
})

// Create Vibe Task (simplified - vibe tasks always start immediately and don't support branches)
export const CreateVibeTaskRequestSchema = z.object({
  agentType: z.string(),
  templateId: z.string(),
  message: z.string().optional(),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional(), // Absolute file paths for uploaded files
  start: z.boolean().optional().default(true),
  name: z.string().optional(), // URL-friendly task name (alphanumeric and dashes only)
  description: z.string().optional(), // One-sentence summary of the task
})

// Additional schemas for task operations
export const MergeTaskSchema = z.object({
  targetBranch: z.string().optional().default('main'),
  commitMessage: z.string().optional(),
})

export const MergeTaskResponseSchema = z.object({
  success: z.boolean(),
  mergeCommit: z.string(),
  sourceBranch: z.string(),
  targetBranch: z.string(),
})

// Derive types from schemas
export type FileQuery = z.infer<typeof FileQuerySchema>
export type TaskFileSearch = z.infer<typeof TaskFileSearchSchema>
export type CommandExecution = z.infer<typeof CommandExecutionSchema>
export type ServerLogsQuery = z.infer<typeof ServerLogsQuerySchema>
export type FileContent = z.infer<typeof FileContentSchema>
export type TaskCommand = z.infer<typeof TaskCommandSchema>
export type TaskMessage = z.infer<typeof TaskMessageSchema>
export type TaskIteration = z.infer<typeof TaskIterationSchema>
export type TaskResponse = z.infer<typeof TaskResponseSchema>
export type TaskListResponse = z.infer<typeof TaskListResponseSchema>
export type CloseTaskResponse = z.infer<typeof CloseTaskResponseSchema>
export type FileListResponse = z.infer<typeof FileListResponseSchema>
export type FileContentResponse = z.infer<typeof FileContentResponseSchema>
export type FileWriteResponse = z.infer<typeof FileWriteResponseSchema>
export type FileSearchResponse = z.infer<typeof FileSearchResponseSchema>
export type GitDiffResponse = z.infer<typeof GitDiffResponseSchema>
export type ServerStatusEnum = z.infer<typeof ServerStatusEnumSchema>
export type ServerStatus = z.infer<typeof ServerStatusSchema>
export type ServerStopResponse = z.infer<typeof ServerStopResponseSchema>
export type ServerLogsResponseExtended = z.infer<typeof ServerLogsResponseExtendedSchema>
export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>
export type CreateVibeTaskRequest = z.infer<typeof CreateVibeTaskRequestSchema>
export type MergeTask = z.infer<typeof MergeTaskSchema>
export type MergeTaskResponse = z.infer<typeof MergeTaskResponseSchema>
export type GitDiffFile = z.infer<typeof GitDiffFileSchema>
// Note: GitStatus, WorkspaceInfo, EnvironmentInfo, AgentInfo are now exported from workspace.schemas.ts
