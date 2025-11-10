import { z } from 'zod'

export const FileQuerySchema = z.object({
  path: z.string().optional().default('/'),
  recursive: z.coerce.boolean().optional().default(false),
})

export const FileSearchSchema = z.object({
  query: z.string(),
  limit: z.number().optional(),
})

export const FileContentSchema = z.object({
  content: z.string(),
})

export const FileContentResponseSchema = z.object({
  content: z.string(),
  encoding: z.string(),
  oldContent: z.string().optional(),
  additions: z.number().optional(),
  deletions: z.number().optional(),
  isModified: z.boolean().optional(),
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

export const FileSearchResponseSchema = z.object({
  files: z.array(
    z.object({
      path: z.string(),
      matches: z.number(),
    }),
  ),
  totalMatches: z.number(),
})

z.object({
  success: z.boolean(),
})

export type FileContentResponse = z.infer<typeof FileContentResponseSchema>
export type FileSearchResponse = z.infer<typeof FileSearchResponseSchema>
