import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { TaskStatusSchema } from './base.schemas.js'
import {
  AgentInfoSchema,
  CapabilitiesSchema,
  EnvironmentInfoSchema,
  GitStatusSchema,
  WorkspaceInfoSchema,
} from './workspace.schemas.js'

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
  durationMs: z.number().optional(),
  gitDiff: z.string().optional(),
  commitHash: z.string().optional(),
  merged: z.boolean().optional(),
  agentSessionId: z.string().optional(),
  model: z.string().optional(),
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
  error: z
    .object({
      message: z.string(),
      stack: z.string().optional(),
      timestamp: dateSchema,
    })
    .optional(),
  willNotResume: z.boolean().optional(),
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
  // Optional workspace status fields that follow dependency hierarchy
  workspace: WorkspaceInfoSchema.optional(), // Only present if task is not closed
  environment: EnvironmentInfoSchema.optional(), // Only present if workspace exists
  agent: AgentInfoSchema.optional(), // Only present if environment is ready
  gitStatus: GitStatusSchema.optional(), // Only when workspace exists
  capabilities: CapabilitiesSchema.optional(), // Only when environment is ready
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

export const CommandResultSchema = z.object({
  command: z.string(),
  output: z.string(),
  exitCode: z.number(),
  durationMs: z.number(),
})

export const ServerStatusSchema = z.object({
  running: z.boolean(),
  configured: z.boolean(),
  url: z.string().optional(),
  pid: z.number().optional(),
  uptimeSeconds: z.number(),
  error: z.string().optional(),
  logs: z.string().optional(),
  responsive: z.boolean().optional(),
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
export type TaskIteration = z.infer<typeof TaskIterationSchema>
export type TaskResponse = z.infer<typeof TaskResponseSchema>
export type TaskListResponse = z.infer<typeof TaskListResponseSchema>
export type CloseTaskResponse = z.infer<typeof CloseTaskResponseSchema>
export type FileListResponse = z.infer<typeof FileListResponseSchema>
export type FileContentResponse = z.infer<typeof FileContentResponseSchema>
export type FileWriteResponse = z.infer<typeof FileWriteResponseSchema>
export type FileSearchResponse = z.infer<typeof FileSearchResponseSchema>
export type GitDiffResponse = z.infer<typeof GitDiffResponseSchema>
export type CommandResult = z.infer<typeof CommandResultSchema>
export type ServerStatus = z.infer<typeof ServerStatusSchema>
export type ServerStopResponse = z.infer<typeof ServerStopResponseSchema>
export type ServerLogsResponseExtended = z.infer<typeof ServerLogsResponseExtendedSchema>
export type MergeTask = z.infer<typeof MergeTaskSchema>
export type MergeTaskResponse = z.infer<typeof MergeTaskResponseSchema>
export type GitDiffFile = z.infer<typeof GitDiffFileSchema>
// Note: GitStatus, WorkspaceInfo, EnvironmentInfo, AgentInfo are now exported from workspace.schemas.ts
