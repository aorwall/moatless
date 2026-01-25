import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { TaskStatusSchema } from './base.schemas.js'
import { MessageContextSchema } from './message.schemas.js'

export const TaskCommandSchema = z.object({
  commandName: z.string(),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  fileIds: z.array(z.string()).optional(),
})

z.object({
  message: z.string(),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  fileIds: z.array(z.string()).optional(),
  agentMode: z.enum(['edit', 'plan']).optional().default('edit'),
})

export const SendMessageRequestSchema = z
  .object({
    message: z.string(),
    resume: z.boolean().optional().default(true),
    model: z.string().optional(),
    fileIds: z.array(z.string()).optional().describe('File IDs from uploaded files'),
    contexts: z
      .array(MessageContextSchema)
      .optional()
      .describe('Context items to provide additional information to the agent'),
    commandName: z.string().optional(),
    agentMode: z.enum(['edit', 'plan']).optional().default('edit'),
    background: z
      .boolean()
      .optional()
      .describe(
        'Run iteration in background mode: bypass permissions, deny interactive tools, cleanup workspace after completion',
      ),
  })
  .refine(
    (data) =>
      (data.message && data.message.trim().length > 0) ||
      (data.contexts && data.contexts.length > 0),
    {
      message: "Either 'message' or at least one context item must be provided",
    },
  )

export const SendMessageResponseSchema = z.object({
  status: z.string(),
  timestamp: z.string(),
})

z.object({
  status: TaskStatusSchema.optional(),
})

z.object({
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

z.object({
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
  initialMessage: z.string().optional(), // TODO: Remove and make sure we got this in messages instead
  commandName: z.string().optional(),
  result: z.string().optional(),
  agentSessionId: z.string().optional(),
  model: z.string().optional(),
  agentMode: z.enum(['edit', 'plan']).optional(),
  willNotResume: z.boolean().optional(),
  screenshotFileId: z.string().optional(),
  siteUrl: z.string().optional(),
  // Token usage fields
  inputTokens: z.number().optional(),
  outputTokens: z.number().optional(),
  // Baseline values for delta calculation (set for resumed sessions)
  baselineCostUsd: z.number().optional(), // TODO: Investigate removal - used for delta calculation in resumed sessions
  baselineInputTokens: z.number().optional(), // TODO: Investigate removal - used for delta calculation in resumed sessions
  baselineOutputTokens: z.number().optional(), // TODO: Investigate removal - used for delta calculation in resumed sessions
  background: z.boolean().optional(),
})

export const TaskResponseSchema = z.object({
  taskId: z.string(),
  shortId: z
    .string()
    .optional()
    .describe('Short alphanumeric ID for external references (e.g., email threading)'),
  createdAt: dateSchema,
  lastMessageAt: dateSchema,
  agentType: z.string(),
  repositoryId: z.string(),
  status: TaskStatusSchema,
  numTurns: z.number(),
  totalCostUsd: z.number(),
  iterations: z.array(TaskIterationSchema),
  templateId: z.string().optional(),
  vibe: z.boolean().optional(),
  name: z.string().optional(),
  description: z.string().optional(),
  siteId: z.string().optional(),
  siteProvider: z.string().optional(),
  targetBranch: z
    .string()
    .optional()
    .describe('Branch for git checkpoint pushes (auto-generated as task/{taskId})'),
  createdBy: z.string().optional().describe('User ID who created this task'),
  createdByName: z.string().optional().describe('Display name of the user who created this task'),
  visibility: z
    .enum(['private', 'public'])
    .default('private')
    .describe(
      "Task visibility: 'private' = owner only, 'public' = all users can read (requires public repository)",
    ),
  title: z.string().optional(),
  repositoryName: z.string().optional(),
  background: z.boolean().optional(),
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

z.object({
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
  fileIds: z.array(z.string()).optional().describe('File IDs from uploaded files'),
  contexts: z
    .array(MessageContextSchema)
    .optional()
    .describe('Context items to provide additional information to the agent'),
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
  targetBranch: z
    .string()
    .optional()
    .describe('Custom target branch name for git pushes (defaults to task/{taskId})'),
  visibility: z
    .enum(['private', 'public'])
    .optional()
    .describe(
      'Task visibility (defaults to repository visibility). Cannot be public if repository is private.',
    ),
  background: z
    .boolean()
    .optional()
    .describe(
      'Run task in background mode: bypass permissions, deny interactive tools, cleanup workspace after completion',
    ),
})

export const CreateVibeTaskRequestSchema = z
  .object({
    agentType: z.string(),
    templateId: z.string(),
    message: z.string().optional(),
    model: z.string().optional(),
    fileIds: z.array(z.string()).optional().describe('File IDs from uploaded files'),
    contexts: z
      .array(MessageContextSchema)
      .optional()
      .describe('Context items to provide additional information to the agent'),
    start: z.boolean().optional().default(true),
    name: z.string().optional().describe('URL-friendly task name (alphanumeric and dashes only)'),
    description: z.string().optional().describe('One-sentence summary of the task'),
    gitCheckpointEnabled: z
      .boolean()
      .optional()
      .describe('Enable automatic git checkpoints after each iteration'),
    gitPushEnabled: z
      .boolean()
      .optional()
      .describe(
        'Enable pushing checkpoints to remote (requires gitCheckpointEnabled and remoteUrl)',
      ),
  })
  .refine(
    (data) =>
      (data.message && data.message.trim().length > 0) ||
      (data.contexts && data.contexts.length > 0),
    {
      message: "Either 'message' or at least one context item must be provided",
    },
  )

z.object({
  targetBranch: z.string().optional().default('main'),
  commitMessage: z.string().optional(),
})

z.object({
  success: z.boolean(),
  mergeCommit: z.string(),
  sourceBranch: z.string(),
  targetBranch: z.string(),
})
export const UpdateTaskVisibilityRequestSchema = z.object({
  visibility: z.enum(['private', 'public']),
})

export const UpdateTaskVisibilityResponseSchema = z.object({
  success: z.literal(true),
})

export type UpdateTaskVisibilityRequest = z.infer<typeof UpdateTaskVisibilityRequestSchema>
export type UpdateTaskVisibilityResponse = z.infer<typeof UpdateTaskVisibilityResponseSchema>

export type SendMessageResponse = z.infer<typeof SendMessageResponseSchema>
export type SimpleTaskResponse = z.infer<typeof SimpleTaskResponseSchema>
export type TaskCommandsResponse = z.infer<typeof TaskCommandsResponseSchema>
export type TaskIteration = z.infer<typeof TaskIterationSchema>
export type TaskResponse = z.infer<typeof TaskResponseSchema>
export type TaskListResponse = z.infer<typeof TaskListResponseSchema>
export type GitDiffResponse = z.infer<typeof GitDiffResponseSchema>
export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>
export type CreateVibeTaskRequest = z.infer<typeof CreateVibeTaskRequestSchema>
