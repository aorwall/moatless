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
  initialMessage: z.string().optional(),
  commandName: z.string().optional(),
  result: z.string().optional(),
  durationMs: z.number().optional(),
  gitDiff: z.string().optional(),
  merged: z.boolean().optional(),
  agentSessionId: z.string().optional(),
  model: z.string().optional(),
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

export const GitDiffResponseSchema = z.object({
  diff: z.string(),
  files: z.array(z.string()),
  hasChanges: z.boolean(),
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
