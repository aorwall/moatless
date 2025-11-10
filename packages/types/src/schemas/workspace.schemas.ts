import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

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

export const GitSyncStatusSchema = z.enum([
  'synced',
  'ahead',
  'behind',
  'diverged',
  'uncommitted_changes',
  'error',
])

export const GitChangesSchema = z.object({
  currentBranch: z.string(),
  status: z.string().describe("Simple status like 'clean' or 'modified'"),
  additions: z.number().describe('Lines added in working directory'),
  deletions: z.number().describe('Lines deleted in working directory'),
  modifications: z.number().describe('Files modified in working directory'),
})

export const WorkspaceTypeSchema = z.enum(['local', 'docker'])

export const EnvironmentStatusSchema = z.enum([
  'not_created',
  'initializing',
  'ready',
  'cleaning',
  'error',
])

export const WorkspaceExistenceSchema = z.enum(['exists', 'missing', 'invalid'])

export const WorkspaceChangeStatusSchema = z.enum([
  'no_changes',
  'merged',
  'committed_unmerged',
  'uncommitted',
  'conflicts',
])

export const AgentStateSchema = z.enum(['idle', 'running', 'none'])

export const BaseWorkspaceSchema = z.object({
  id: z.string(),
  type: WorkspaceTypeSchema,
  repositoryId: z.string(),
  status: EnvironmentStatusSchema,
  createdAt: dateSchema,
  error: z.string().optional(),
})

export const LocalWorkspaceSchema = BaseWorkspaceSchema.extend({
  type: z.literal('local'),
  path: z.string(),
  branch: z.string().optional(),
  originalBranch: z.string().optional(),
})

export const DockerWorkspaceSchema = BaseWorkspaceSchema.extend({
  type: z.literal('docker'),
  containerName: z.string(),
  port: z.number().optional(),
  image: z.string().optional(),
})

export const WorkspaceSchema = z.union([LocalWorkspaceSchema, DockerWorkspaceSchema])

export const WorkspaceFileSchema = z.object({
  path: z.string(),
  content: z.string(),
  language: z.string().optional(),
})

export const WorkspaceChangeSchema = z.object({
  type: z.enum(['create', 'modify', 'delete']),
  path: z.string(),
  oldContent: z.string().optional(),
  newContent: z.string().optional(),
})

export const WorkspaceFileSearchResultSchema = z.object({
  path: z.string(),
  matches: z.number(),
})

export const WorkspaceCapabilitiesSchema = z.object({
  canListFiles: z.boolean(),
  canReadWriteFiles: z.boolean(),
  canSearchFiles: z.boolean(),
  canExecuteCode: z.boolean(),
})

export const GitChangeStatusSchema = z.enum(['added', 'modified', 'deleted', 'renamed'])

// Type definition for FileTreeNode will be inferred from the schema
export type FileTreeNodeType = {
  name: string
  path: string
  type: 'file' | 'directory'
  isChanged: boolean
  gitStatus?: z.infer<typeof GitChangeStatusSchema> | undefined
  additions?: number | undefined
  deletions?: number | undefined
  isStaged?: boolean | undefined
  children?: FileTreeNodeType[] | undefined
}

export const FileTreeNodeSchema: z.ZodType<FileTreeNodeType> = z
  .lazy(() =>
    z.object({
      name: z.string(),
      path: z.string(),
      type: z.enum(['file', 'directory']),
      isChanged: z.boolean(),
      gitStatus: GitChangeStatusSchema.optional(),
      additions: z.number().optional(),
      deletions: z.number().optional(),
      isStaged: z.boolean().optional(),
      children: z.array(FileTreeNodeSchema).optional(),
    }),
  )
  .openapi({
    type: 'object',
    properties: {
      name: { type: 'string' },
      path: { type: 'string' },
      type: { type: 'string', enum: ['file', 'directory'] },
      isChanged: { type: 'boolean' },
      gitStatus: {
        type: 'string',
        enum: ['added', 'modified', 'deleted', 'renamed'],
      },
      additions: { type: 'number' },
      deletions: { type: 'number' },
      isStaged: { type: 'boolean' },
      children: {
        type: 'array',
        items: { $ref: '#/components/schemas/FileTreeNode' },
      },
    },
    required: ['name', 'path', 'type', 'isChanged'],
  })

export const FileTreeWithChangesResponseSchema = z.object({
  tree: z.array(FileTreeNodeSchema),
  gitSummary: z.object({
    totalFiles: z.number(),
    totalAdditions: z.number(),
    totalDeletions: z.number(),
    totalModified: z.number(),
    totalStaged: z.number(),
    compareBase: z.string(),
    hasChanges: z.boolean(),
  }),
})

export const WorkspaceSetupRequestSchema = z.object({})

export const WorkspaceSetupResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
  status: EnvironmentStatusSchema,
})

export const WorkspaceRefreshConfigRequestSchema = z.object({
  restartServer: z.boolean().optional().default(false),
})

export const WorkspaceRefreshConfigResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
  status: EnvironmentStatusSchema,
  serverRestarted: z.boolean().optional(),
})

export type Workspace = z.infer<typeof WorkspaceSchema>
export type FileTreeNode = z.infer<typeof FileTreeNodeSchema>
export type FileTreeWithChangesResponse = z.infer<typeof FileTreeWithChangesResponseSchema>
export type GitStatus = z.infer<typeof GitStatusSchema>
export type GitSyncStatus = z.infer<typeof GitSyncStatusSchema>
export type GitChanges = z.infer<typeof GitChangesSchema>

export const WorkspaceInfoSchema = z.object({
  state: WorkspaceExistenceSchema,
  path: z.string(),
  changeStatus: WorkspaceChangeStatusSchema.optional(),
})

export const EnvironmentInfoSchema = z.object({
  type: z.enum(['local', 'docker']),
  status: EnvironmentStatusSchema,
  error: z.string().optional(),
  createdAt: z.string(),
})

export const AgentInfoSchema = z.object({
  state: AgentStateSchema,
  type: z.string().optional(),
})

export const CapabilitiesSchema = z.object({
  fileOperations: z.boolean(),
  commandExecution: z.boolean(),
  serverManagement: z.boolean(),
})

export const WorkspaceStatusResponseSchema = z.object({
  workspaceState: WorkspaceExistenceSchema,
  workspacePath: z.string(),
  workspaceChangeStatus: WorkspaceChangeStatusSchema.optional(),
  environmentType: z.enum(['local', 'docker']),
  environmentStatus: EnvironmentStatusSchema,
  environmentError: z.string().optional(),
  environmentCreatedAt: z.string(),
  repositoryUpdatedAt: z.string().optional(),
  configOutdated: z.boolean().default(false),
  agentState: AgentStateSchema.or(z.literal('none')).default('none'),
  agentType: z.string().optional(),
  gitStatus: GitStatusSchema.optional(),
  capabilities: CapabilitiesSchema,
  isRunning: z.boolean(),
  workspaceReady: z.boolean(),
  canExecute: z.boolean(),
  logFilePath: z.string().optional(),
})

export type WorkspaceStatusResponse = z.infer<typeof WorkspaceStatusResponseSchema>

z.object({
  success: z.boolean().describe('Whether the sync was successful'),
  conflicts: z.boolean().describe('Whether conflicts were detected'),
  message: z.string().describe('Description of the sync result'),
  commitsBehind: z.number().describe('Number of commits workspace is behind').optional(),
  commitsAhead: z.number().describe('Number of commits workspace is ahead').optional(),
  filesWithConflicts: z.array(z.string()).describe('Files that have conflicts').optional(),
  unmergedPaths: z.array(z.string()).describe('Paths that could not be merged').optional(),
})

export const CommandExecutionOptionsSchema = z.object({
  commands: z.array(z.string()).min(1),
  purpose: z
    .enum(['setup', 'test', 'build', 'lint', 'deps', 'custom'])
    .optional()
    .default('custom'),
  continueOnError: z.boolean().optional().default(false),
  timeout: z
    .number()
    .optional()
    .default(300000)
    .describe('Timeout in milliseconds (default: 5 minutes)'),
  workingDirectory: z.string().optional(),
})
