import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { TaskStatusSchema } from './base.schemas.js'

export const TaskCommandSchema = z.object({
  commandName: z.string(),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional(),
})

export const TaskMessageSchema = z.object({
  message: z.string(),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional(),
  agentMode: z.enum(['edit', 'plan']).optional().default('edit'),
})

export const SendMessageRequestSchema = z.object({
  message: z.string().min(1),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional().describe('Absolute file paths for uploaded files'),
  commandName: z.string().optional(),
  agentMode: z.enum(['edit', 'plan']).optional().default('edit'),
})

export const SendMessageResponseSchema = z.object({
  status: z.string(),
  timestamp: z.string(),
})

export const UpdateTaskRequestSchema = z.object({
  status: TaskStatusSchema.optional(),
})

export const UpdateTaskResponseSchema = z.object({
  status: z.string(),
})

export const SimpleTaskResponseSchema = z.object({
  taskId: z.string(),
  status: z.string(),
  task: z.lazy(() => TaskResponseSchema).optional(),
})

export const DeleteTaskResponseSchema = z.object({
  status: z.string(),
})

export const StopAgentResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
})

export const ServerLogsResponseSchema = z.object({
  logs: z.string(),
  lines: z.number(),
})

export const TaskCommandsResponseSchema = z.object({
  commands: z.array(
    z.object({
      name: z.string(),
      description: z.string(),
      prompt: z.string(),
      conditions: z.any().optional(),
      icon: z.string().optional(),
      variant: z.string().optional(),
      active: z.boolean().optional(),
    }),
  ),
})

export const StartTaskResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
})

export const CloseTaskResponseSchemaAPI = z.object({
  status: z.string(),
  taskId: z.string(),
})

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
  willNotResume: z.boolean().optional(),
  screenshotPath: z.string().optional(),
  thumbnailPath: z.string().optional(),
  siteUrl: z.string().optional(),
  checkpointCommitHash: z.string().optional(),
  checkpointPushed: z.boolean().optional(),
})

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
  vibe: z.boolean().optional(),
  name: z.string().optional(),
  description: z.string().optional(),
  siteId: z.string().optional(),
  siteProvider: z.string().optional(),
  gitCheckpointEnabled: z.boolean().optional(),
  gitPushEnabled: z.boolean().optional(),
  title: z.string().optional(),
  repositoryName: z.string().optional(),
})

export const TaskListResponseSchema = z.object({
  items: z.array(TaskResponseSchema),
  pagination: z.object({
    limit: z.number(),
    offset: z.number(),
    total: z.number().or(z.undefined()),
    hasMore: z.boolean().or(z.undefined()),
  }),
})

export const CloseTaskResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
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

export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  repositoryId: z.string(),
  message: z.string().optional(),
  branchName: z.string().optional().describe('Deprecated, use originalBranch'),
  originalBranch: z.string().optional().describe('Branch to create from (the source branch)'),
  newBranch: z
    .string()
    .optional()
    .describe('When set, create branch with this name and also worktree if using worktree'),
  start: z.boolean().optional(),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional().describe('Absolute file paths for uploaded files'),
  agentMode: z.enum(['edit', 'plan']).optional().describe('Agent mode for task creation'),
  templateId: z.string().optional().describe('Template ID when created from template'),
  name: z.string().optional().describe('URL-friendly task name (alphanumeric and dashes only)'),
  description: z.string().optional().describe('One-sentence summary of the task'),
  gitCheckpointEnabled: z
    .boolean()
    .optional()
    .describe('Enable automatic git checkpoints after each iteration'),
  gitPushEnabled: z
    .boolean()
    .optional()
    .describe('Enable pushing checkpoints to remote (requires gitCheckpointEnabled and remoteUrl)'),
})

export const CreateVibeTaskRequestSchema = z.object({
  agentType: z.string(),
  templateId: z.string(),
  message: z.string().optional(),
  model: z.string().optional(),
  filePaths: z.array(z.string()).optional().describe('Absolute file paths for uploaded files'),
  start: z.boolean().optional().default(true),
  name: z.string().optional().describe('URL-friendly task name (alphanumeric and dashes only)'),
  description: z.string().optional().describe('One-sentence summary of the task'),
})

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
export type TaskCommand = z.infer<typeof TaskCommandSchema>
export type TaskMessage = z.infer<typeof TaskMessageSchema>
export type SendMessageRequest = z.infer<typeof SendMessageRequestSchema>
export type SendMessageResponse = z.infer<typeof SendMessageResponseSchema>
export type UpdateTaskRequest = z.infer<typeof UpdateTaskRequestSchema>
export type UpdateTaskResponse = z.infer<typeof UpdateTaskResponseSchema>
export type SimpleTaskResponse = z.infer<typeof SimpleTaskResponseSchema>
export type DeleteTaskResponse = z.infer<typeof DeleteTaskResponseSchema>
export type StopAgentResponse = z.infer<typeof StopAgentResponseSchema>
export type ServerLogsResponse = z.infer<typeof ServerLogsResponseSchema>
export type TaskCommandsResponse = z.infer<typeof TaskCommandsResponseSchema>
export type StartTaskResponse = z.infer<typeof StartTaskResponseSchema>
export type TaskIteration = z.infer<typeof TaskIterationSchema>
export type TaskResponse = z.infer<typeof TaskResponseSchema>
export type TaskListResponse = z.infer<typeof TaskListResponseSchema>
export type CloseTaskResponse = z.infer<typeof CloseTaskResponseSchema>
export type GitDiffResponse = z.infer<typeof GitDiffResponseSchema>
export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>
export type CreateVibeTaskRequest = z.infer<typeof CreateVibeTaskRequestSchema>
export type MergeTask = z.infer<typeof MergeTaskSchema>
export type MergeTaskResponse = z.infer<typeof MergeTaskResponseSchema>
export type GitDiffFile = z.infer<typeof GitDiffFileSchema>
