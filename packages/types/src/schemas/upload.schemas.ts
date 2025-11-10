import { z } from 'zod'

z.object({
  file: z.instanceof(File),
})

export const UploadFileResponseSchema = z.object({
  fileId: z.string(),
  filename: z.string(),
  mimeType: z.string(),
  size: z.number(),
  uploadedAt: z.string(),
})

export const UploadFileIdParamsSchema = z.object({
  fileId: z.string().min(1),
})

// Deprecated - for backward compatibility
z.object({
  uploadId: z.string().min(1),
  filename: z.string().min(1),
})

export type UploadFileResponse = z.infer<typeof UploadFileResponseSchema>
