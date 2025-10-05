import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

// ============================================
// Shared Status Component Schemas
// ============================================

// Git Status Schema (shared between workspace and task responses)
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

// ============================================
// Workspace Enums and Base Types
// ============================================

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

// ============================================
// Workspace Schemas
// ============================================

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

// ============================================
// File and Change Schemas
// ============================================

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

// ============================================
// Search and File Operations
// ============================================

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

// ============================================
// Server Management (re-use from task schemas)
// ============================================

// CommandResult and ServerStatus are already defined in task.schemas.ts
// We re-export them here for consistency

// ============================================
// File Tree with Git Changes Schema
// ============================================

export const GitChangeStatusSchema = z.enum(['added', 'modified', 'deleted', 'renamed'])

// Type definition for FileTreeNode will be inferred from the schema
export type FileTreeNodeType = {
  name: string
  path: string
  type: 'file' | 'directory'
  isChanged: boolean
  gitStatus?: z.infer<typeof GitChangeStatusSchema>
  additions?: number
  deletions?: number
  isStaged?: boolean
  children?: FileTreeNodeType[]
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
      gitStatus: { type: 'string', enum: ['added', 'modified', 'deleted', 'renamed'] },
      additions: { type: 'number' },
      deletions: { type: 'number' },
      isStaged: { type: 'boolean' },
      children: { type: 'array', items: { $ref: '#/components/schemas/FileTreeNode' } },
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

// ============================================
// Workspace Setup Schemas
// ============================================

export const WorkspaceSetupRequestSchema = z.object({})

export const WorkspaceSetupResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
  status: EnvironmentStatusSchema,
})

// ============================================
// Workspace Refresh Config Schemas
// ============================================

export const WorkspaceRefreshConfigRequestSchema = z.object({
  restartServer: z.boolean().optional().default(false),
})

export const WorkspaceRefreshConfigResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
  status: EnvironmentStatusSchema,
  serverRestarted: z.boolean().optional(),
})

// ============================================
// Derived Types
// ============================================

export type WorkspaceType = z.infer<typeof WorkspaceTypeSchema>
export type EnvironmentStatus = z.infer<typeof EnvironmentStatusSchema>
export type WorkspaceExistence = z.infer<typeof WorkspaceExistenceSchema>
export type WorkspaceChangeStatus = z.infer<typeof WorkspaceChangeStatusSchema>
export type AgentState = z.infer<typeof AgentStateSchema>
export type BaseWorkspace = z.infer<typeof BaseWorkspaceSchema>
export type LocalWorkspace = z.infer<typeof LocalWorkspaceSchema>
export type DockerWorkspace = z.infer<typeof DockerWorkspaceSchema>
export type Workspace = z.infer<typeof WorkspaceSchema>
export type WorkspaceFile = z.infer<typeof WorkspaceFileSchema>
export type WorkspaceChange = z.infer<typeof WorkspaceChangeSchema>
export type WorkspaceFileSearchResult = z.infer<typeof WorkspaceFileSearchResultSchema>
export type WorkspaceCapabilities = z.infer<typeof WorkspaceCapabilitiesSchema>
export type GitChangeStatus = z.infer<typeof GitChangeStatusSchema>
export type FileTreeNode = z.infer<typeof FileTreeNodeSchema>
export type FileTreeWithChangesResponse = z.infer<typeof FileTreeWithChangesResponseSchema>
export type WorkspaceSetupRequest = z.infer<typeof WorkspaceSetupRequestSchema>
export type WorkspaceSetupResponse = z.infer<typeof WorkspaceSetupResponseSchema>
export type WorkspaceRefreshConfigRequest = z.infer<typeof WorkspaceRefreshConfigRequestSchema>
export type WorkspaceRefreshConfigResponse = z.infer<typeof WorkspaceRefreshConfigResponseSchema>

// Shared Status Component Types
export type GitStatus = z.infer<typeof GitStatusSchema>
export type WorkspaceInfo = z.infer<typeof WorkspaceInfoSchema>
export type EnvironmentInfo = z.infer<typeof EnvironmentInfoSchema>
export type AgentInfo = z.infer<typeof AgentInfoSchema>
export type Capabilities = z.infer<typeof CapabilitiesSchema>

// Workspace Info (for status responses)
export const WorkspaceInfoSchema = z.object({
  state: WorkspaceExistenceSchema,
  path: z.string(),
  changeStatus: WorkspaceChangeStatusSchema.optional(),
})

// Environment Info (for status responses)
export const EnvironmentInfoSchema = z.object({
  type: z.enum(['local', 'docker']),
  status: EnvironmentStatusSchema,
  error: z.string().optional(),
  createdAt: z.string(),
})

// Agent Info (for status responses)
export const AgentInfoSchema = z.object({
  state: AgentStateSchema,
  type: z.string().optional(),
})

// Capabilities (for status responses)
export const CapabilitiesSchema = z.object({
  fileOperations: z.boolean(),
  commandExecution: z.boolean(),
  serverManagement: z.boolean(),
})

// ============================================
// Workspace Status Response Schema
// ============================================

// Workspace Status Response with flattened structure
export const WorkspaceStatusResponseSchema = z.object({
  // Workspace info
  workspaceState: WorkspaceExistenceSchema,
  workspacePath: z.string(),
  workspaceChangeStatus: WorkspaceChangeStatusSchema.optional(),

  // Environment info
  environmentType: z.enum(['local', 'docker']),
  environmentStatus: EnvironmentStatusSchema,
  environmentError: z.string().optional(),
  environmentCreatedAt: z.string(),

  // Repository config info
  repositoryUpdatedAt: z.string().optional(),
  configOutdated: z.boolean().default(false),

  // Agent info
  agentState: AgentStateSchema.or(z.literal('none')).default('none'),
  agentType: z.string().optional(),

  // Git status (already flat)
  gitStatus: GitStatusSchema.optional(),

  // Capabilities (already flat)
  capabilities: CapabilitiesSchema,

  // Computed states
  isRunning: z.boolean(),
  workspaceReady: z.boolean(),
  canExecute: z.boolean(),

  // Logging
  logFilePath: z.string().optional(),
})

export type WorkspaceStatusResponse = z.infer<typeof WorkspaceStatusResponseSchema>

// ============================================
// Workspace Sync Schemas (moved from api.schemas.ts)
// ============================================

// Sync with Main Response Schema
export const SyncWorkspaceResponseSchema = z.object({
  success: z.boolean().describe('Whether the sync was successful'),
  conflicts: z.boolean().describe('Whether conflicts were detected'),
  message: z.string().describe('Description of the sync result'),
  commitsBehind: z.number().describe('Number of commits workspace is behind').optional(),
  commitsAhead: z.number().describe('Number of commits workspace is ahead').optional(),
  filesWithConflicts: z.array(z.string()).describe('Files that have conflicts').optional(),
  unmergedPaths: z.array(z.string()).describe('Paths that could not be merged').optional(),
})

export type SyncWorkspaceResponse = z.infer<typeof SyncWorkspaceResponseSchema>

// ============================================
// Command Execution Schemas
// ============================================

export const CommandExecutionOptionsSchema = z.object({
  commands: z.array(z.string()).min(1),
  purpose: z
    .enum(['setup', 'test', 'build', 'lint', 'deps', 'custom'])
    .optional()
    .default('custom'),
  continueOnError: z.boolean().optional().default(false),
  timeout: z.number().optional().default(300000), // 5 minutes default
  workingDirectory: z.string().optional(),
})

// Export types
export type CommandExecutionOptions = z.infer<typeof CommandExecutionOptionsSchema>
