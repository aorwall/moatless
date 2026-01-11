import { z } from 'zod'

/**
 * Read file request schema
 */
export const ReadFileRequestSchema = z.object({
  filePath: z.string().min(1),
})

export type ReadFileRequest = z.infer<typeof ReadFileRequestSchema>

/**
 * Read file response schema
 */
export const ReadFileResponseSchema = z.object({
  content: z.string(),
})

export type ReadFileResponse = z.infer<typeof ReadFileResponseSchema>

/**
 * Write file request schema
 */
export const WriteFileRequestSchema = z.object({
  filePath: z.string().min(1),
  content: z.string(),
})

export type WriteFileRequest = z.infer<typeof WriteFileRequestSchema>

/**
 * Write file response schema
 */
export const WriteFileResponseSchema = z.object({
  success: z.boolean(),
  error: z.string().optional(),
})

export type WriteFileResponse = z.infer<typeof WriteFileResponseSchema>

/**
 * List files request schema
 */
export const ListFilesRequestSchema = z.object({
  relativePath: z.string().optional(),
  recursive: z.boolean().optional(),
})

export type ListFilesRequest = z.infer<typeof ListFilesRequestSchema>

/**
 * List files response schema
 */
export const ListFilesResponseSchema = z.object({
  files: z.array(z.string()),
  error: z.string().optional(),
})

export type ListFilesResponse = z.infer<typeof ListFilesResponseSchema>

/**
 * Search files request schema
 */
export const SearchFilesRequestSchema = z.object({
  query: z.string().min(1),
  limit: z.number().optional(),
})

export type SearchFilesRequest = z.infer<typeof SearchFilesRequestSchema>

/**
 * File match result schema
 */
export const FileMatchSchema = z.object({
  path: z.string(),
  matches: z.number(),
})

export type FileMatch = z.infer<typeof FileMatchSchema>

/**
 * Search files response schema
 */
export const SearchFilesResponseSchema = z.object({
  files: z.array(FileMatchSchema),
  totalMatches: z.number(),
  error: z.string().optional(),
})

export type SearchFilesResponse = z.infer<typeof SearchFilesResponseSchema>

/**
 * Read file with changes request schema
 */
export const ReadFileWithChangesRequestSchema = z.object({
  filePath: z.string().min(1),
})

export type ReadFileWithChangesRequest = z.infer<typeof ReadFileWithChangesRequestSchema>

/**
 * Read file with changes response schema
 */
export const ReadFileWithChangesResponseSchema = z.object({
  content: z.string(),
  oldContent: z.string().optional(),
  additions: z.number().optional(),
  deletions: z.number().optional(),
  isModified: z.boolean().optional(),
  error: z.string().optional(),
})

export type ReadFileWithChangesResponse = z.infer<typeof ReadFileWithChangesResponseSchema>

/**
 * File tree request schema
 */
export const FileTreeRequestSchema = z.object({
  rootPath: z.string().optional(),
})

export type FileTreeRequest = z.infer<typeof FileTreeRequestSchema>
