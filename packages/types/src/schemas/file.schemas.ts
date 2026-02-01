import { z } from 'zod'

export const FileSearchSchema = z.object({
  query: z.string(),
  limit: z.number().optional(),
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

export type FileSearchResponse = z.infer<typeof FileSearchResponseSchema>
