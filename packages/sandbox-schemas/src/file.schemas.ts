import { z } from 'zod'

export const ReadFileRequestSchema = z.object({
  filePath: z.string().min(1),
})

export type ReadFileRequest = z.infer<typeof ReadFileRequestSchema>

export const ReadFileResponseSchema = z.object({
  content: z.string(),
})

export type ReadFileResponse = z.infer<typeof ReadFileResponseSchema>

export const WriteFileRequestSchema = z.object({
  filePath: z.string().min(1),
  content: z.string(),
})

export type WriteFileRequest = z.infer<typeof WriteFileRequestSchema>

export const WriteFileResponseSchema = z.object({
  success: z.boolean(),
  error: z.string().optional(),
})

export type WriteFileResponse = z.infer<typeof WriteFileResponseSchema>

export const ListFilesRequestSchema = z.object({
  relativePath: z.string().optional(),
  recursive: z.boolean().optional(),
})

export type ListFilesRequest = z.infer<typeof ListFilesRequestSchema>

export const ListFilesResponseSchema = z.object({
  files: z.array(z.string()),
  error: z.string().optional(),
})

export type ListFilesResponse = z.infer<typeof ListFilesResponseSchema>

export const SearchFilesRequestSchema = z.object({
  query: z.string().min(1),
  limit: z.number().optional(),
})

export type SearchFilesRequest = z.infer<typeof SearchFilesRequestSchema>

export const FileMatchSchema = z.object({
  path: z.string(),
  matches: z.number(),
})

export type FileMatch = z.infer<typeof FileMatchSchema>

export const SearchFilesResponseSchema = z.object({
  files: z.array(FileMatchSchema),
  totalMatches: z.number(),
  error: z.string().optional(),
})

export type SearchFilesResponse = z.infer<typeof SearchFilesResponseSchema>

export const ReadFileWithChangesRequestSchema = z.object({
  filePath: z.string().min(1),
})

export type ReadFileWithChangesRequest = z.infer<typeof ReadFileWithChangesRequestSchema>

export const ReadFileWithChangesResponseSchema = z.object({
  content: z.string(),
  oldContent: z.string().optional(),
  additions: z.number().optional(),
  deletions: z.number().optional(),
  isModified: z.boolean().optional(),
  error: z.string().optional(),
})

export type ReadFileWithChangesResponse = z.infer<typeof ReadFileWithChangesResponseSchema>

export const FileTreeRequestSchema = z.object({
  rootPath: z.string().optional(),
})

export type FileTreeRequest = z.infer<typeof FileTreeRequestSchema>
