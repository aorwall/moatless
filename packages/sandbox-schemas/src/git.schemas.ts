import { z } from 'zod'

/**
 * Git sync status enum
 */
export const GitSyncStatusSchema = z.enum([
  'synced',
  'ahead',
  'behind',
  'diverged',
  'error',
  'unknown',
])

export type GitSyncStatus = z.infer<typeof GitSyncStatusSchema>

/**
 * Git status response schema
 */
export const GitStatusResponseSchema = z.object({
  currentBranch: z.string(),
  baseBranch: z.string(),
  hasUncommittedChanges: z.boolean(),
  hasConflicts: z.boolean(),
  status: GitSyncStatusSchema,
  commitsAhead: z.number(),
  commitsBehind: z.number(),
  originExists: z.boolean(),
  additions: z.number(),
  deletions: z.number(),
  modifications: z.number(),
  error: z.string().optional(),
})

export type GitStatusResponse = z.infer<typeof GitStatusResponseSchema>

/**
 * Git changes response schema
 */
export const GitChangesResponseSchema = z.object({
  currentBranch: z.string(),
  status: GitSyncStatusSchema,
  additions: z.number(),
  deletions: z.number(),
  modifications: z.number(),
  error: z.string().optional(),
})

export type GitChangesResponse = z.infer<typeof GitChangesResponseSchema>

/**
 * Git diff file schema
 */
export const GitDiffFileSchema = z.object({
  oldFile: z.string().optional(),
  newFile: z.string().optional(),
  hunks: z.array(z.string()),
})

export type GitDiffFile = z.infer<typeof GitDiffFileSchema>

/**
 * Git diff response schema
 */
export const GitDiffResponseSchema = z.object({
  diff: z.string().optional(),
  files: z.array(GitDiffFileSchema),
  error: z.string().optional(),
})

export type GitDiffResponse = z.infer<typeof GitDiffResponseSchema>

/**
 * Current commit hash response schema
 */
export const CurrentCommitHashResponseSchema = z.object({
  hash: z.string().nullable(),
})

export type CurrentCommitHashResponse = z.infer<typeof CurrentCommitHashResponseSchema>

/**
 * File tree node schema (recursive)
 */
export const FileTreeNodeSchema: z.ZodType<FileTreeNode> = z.lazy(() =>
  z.object({
    name: z.string(),
    type: z.enum(['file', 'directory']),
    path: z.string(),
    gitStatus: z.enum(['added', 'modified', 'deleted', 'renamed', 'unchanged']).optional(),
    additions: z.number().optional(),
    deletions: z.number().optional(),
    isStaged: z.boolean().optional(),
    children: z.array(FileTreeNodeSchema).optional(),
  }),
)

export interface FileTreeNode {
  name: string
  type: 'file' | 'directory'
  path: string
  gitStatus?: 'added' | 'modified' | 'deleted' | 'renamed' | 'unchanged'
  additions?: number
  deletions?: number
  isStaged?: boolean
  children?: FileTreeNode[]
}

/**
 * Git summary schema
 */
export const GitSummarySchema = z.object({
  totalFiles: z.number(),
  totalAdditions: z.number(),
  totalDeletions: z.number(),
  totalStaged: z.number(),
  totalModified: z.number(),
  compareBase: z.string(),
  hasChanges: z.boolean(),
})

export type GitSummary = z.infer<typeof GitSummarySchema>

/**
 * File tree with changes response schema
 */
export const FileTreeWithChangesResponseSchema = z.object({
  tree: z.array(FileTreeNodeSchema),
  gitSummary: GitSummarySchema,
  error: z.string().optional(),
})

export type FileTreeWithChangesResponse = z.infer<typeof FileTreeWithChangesResponseSchema>
