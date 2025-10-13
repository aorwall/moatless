import { z } from 'zod'

export const UploadFileRequestSchema = z.object({
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
export const UploadPathParamsSchema = z.object({
  uploadId: z.string().min(1),
  filename: z.string().min(1),
})

export type UploadFileRequest = z.infer<typeof UploadFileRequestSchema>
export type UploadFileResponse = z.infer<typeof UploadFileResponseSchema>
export type UploadFileIdParams = z.infer<typeof UploadFileIdParamsSchema>
export type UploadPathParams = z.infer<typeof UploadPathParamsSchema>
