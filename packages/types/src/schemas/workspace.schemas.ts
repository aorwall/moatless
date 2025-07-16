import { z } from 'zod'
import { GitStatusSchema } from './task.schemas.js'

// ============================================
// Workspace Enums and Base Types
// ============================================

export const WorkspaceStatusSchema = z.enum(['not_setup', 'setting_up', 'idle', 'running'])

export const WorkspaceTypeSchema = z.enum(['local', 'docker'])

export const WorkspaceStateSchema = z.enum([
  'not_setup',
  'initializing',
  'ready',
  'error',
  'cleaning',
])

// ============================================
// Workspace Schemas
// ============================================

export const BaseWorkspaceSchema = z.object({
  id: z.string(),
  type: WorkspaceTypeSchema,
  repositoryId: z.string(),
  status: WorkspaceStateSchema,
  createdAt: z.date(),
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
  containerId: z.string(),
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
type FileTreeNodeType = {
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

export const FileTreeNodeSchema: z.ZodType<FileTreeNodeType> = z.lazy(() =>
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
// Derived Types
// ============================================

export type WorkspaceStatus = z.infer<typeof WorkspaceStatusSchema>
export type WorkspaceType = z.infer<typeof WorkspaceTypeSchema>
export type WorkspaceState = z.infer<typeof WorkspaceStateSchema>
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

// ============================================
// Workspace Status Response Schema
// ============================================

// Workspace Status Response
export const WorkspaceStatusResponseSchema = z.object({
  status: z.enum(['not_setup', 'setting_up', 'idle', 'running']),
  gitStatus: GitStatusSchema.optional(),
  backgroundTaskCount: z.number(),
  environment: z.object({
    type: z.enum(['local', 'docker']),
    status: z.enum(['not_setup', 'initializing', 'ready', 'error', 'cleaning']),
    error: z.string().optional(),
    createdAt: z.string(), // ISO date string
    capabilities: z.object({
      fileOperations: z.boolean(),
      commandExecution: z.boolean(),
    }),
  }),
  gitChanges: z
    .object({
      totalChanges: z.number(),
      totalAdditions: z.number(),
      totalDeletions: z.number(),
      files: z.array(z.string()),
    })
    .optional(),
})

export type WorkspaceStatusResponse = z.infer<typeof WorkspaceStatusResponseSchema>

// ============================================
// Helper Functions (these remain as utilities)
// ============================================

export function isLocalWorkspace(workspace: Workspace): workspace is LocalWorkspace {
  return workspace.type === 'local'
}

export function isDockerWorkspace(workspace: Workspace): workspace is DockerWorkspace {
  return workspace.type === 'docker'
}
