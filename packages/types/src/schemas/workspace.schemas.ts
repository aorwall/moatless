import { z } from 'zod'

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

// ============================================
// Helper Functions (these remain as utilities)
// ============================================

export function isLocalWorkspace(workspace: Workspace): workspace is LocalWorkspace {
  return workspace.type === 'local'
}

export function isDockerWorkspace(workspace: Workspace): workspace is DockerWorkspace {
  return workspace.type === 'docker'
}
