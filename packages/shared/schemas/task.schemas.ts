import { z } from 'zod'

// Query schemas
export const EventTypeQuerySchema = z.object({
  type: z.string().optional(),
})

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
  startTime: z.date(),
  endTime: z.date().optional(),
  status: z.enum(['created', 'active', 'completed', 'error', 'failed']),
  messageCount: z.number(),
  costUsd: z.number(),
  contextTokens: z.number().optional(),
  initialMessage: z.string().optional(),
  commandName: z.string().optional(),
  result: z.string().optional(),
  durationMs: z.number().optional(),
  gitDiff: z.string().optional(),
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
      timestamp: z.date(),
    })
    .optional(),
  willNotResume: z.boolean().optional(),
  // Runtime properties (not persisted)
  task: z.any().optional(),
  taskId: z.string().optional(),
})

export const TaskResponseSchema = z.object({
  taskId: z.string(),
  createdAt: z.date(),
  lastMessageAt: z.date(),
  agentType: z.string(),
  repositoryId: z.string(),
  status: z.enum(['open', 'running', 'error', 'merged', 'closed']),
  numTurns: z.number(),
  totalCostUsd: z.number(),
  iterations: z.array(TaskIterationSchema),
  workspacePath: z.string().optional(),
})

export const TaskListResponseSchema = z.object({
  tasks: z.array(TaskResponseSchema),
  items: z.array(TaskResponseSchema),
  pagination: z.object({
    limit: z.number(),
    offset: z.number(),
    total: z.number().optional(),
  }),
})

export const EventListResponseSchema = z.object({
  events: z.array(z.any()), // Event type is complex, using any for now
  items: z.array(z.any()),
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

export const GitStatusSchema = z.object({
  currentBranch: z.string(),
  baseBranch: z.string(),
  hasUncommittedChanges: z.boolean(),
  hasConflicts: z.boolean(),
  status: z.string(),
  commitsAhead: z.number(),
  commitsBehind: z.number(),
  originExists: z.boolean(),
  additions: z.number().optional(),
  deletions: z.number().optional(),
  modifications: z.number().optional(),
})

// Derive types from schemas
export type EventTypeQuery = z.infer<typeof EventTypeQuerySchema>
export type FileQuery = z.infer<typeof FileQuerySchema>
export type TaskFileSearch = z.infer<typeof TaskFileSearchSchema>
export type CommandExecution = z.infer<typeof CommandExecutionSchema>
export type ServerLogsQuery = z.infer<typeof ServerLogsQuerySchema>
export type FileContent = z.infer<typeof FileContentSchema>
export type TaskCommand = z.infer<typeof TaskCommandSchema>
export type TaskIteration = z.infer<typeof TaskIterationSchema>
export type TaskResponse = z.infer<typeof TaskResponseSchema>
export type TaskListResponse = z.infer<typeof TaskListResponseSchema>
export type EventListResponse = z.infer<typeof EventListResponseSchema>
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
export type GitStatus = z.infer<typeof GitStatusSchema>
export type GitDiffFile = z.infer<typeof GitDiffFileSchema>
