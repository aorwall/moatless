import { z } from 'zod'

export const UploadFileRequestSchema = z.object({
  file: z.instanceof(File),
  taskId: z.string().optional(),
})

export const UploadFileResponseSchema = z.object({
  id: z.string(),
  filename: z.string(),
  path: z.string().describe('Relative path like /uploads/{randomId}/filename.ext'),
  absolutePath: z.string().describe('Absolute path in workspace'),
  size: z.number(),
  mimeType: z.string(),
  uploadedAt: z.string(),
})

export const UploadPathParamsSchema = z.object({
  uploadId: z.string().min(1),
  filename: z.string().min(1),
})

export type UploadFileRequest = z.infer<typeof UploadFileRequestSchema>
export type UploadFileResponse = z.infer<typeof UploadFileResponseSchema>
export type UploadPathParams = z.infer<typeof UploadPathParamsSchema>
