import { z } from 'zod'

// ============================================
// Request Schemas
// ============================================

// Upload file request body (multipart/form-data)
export const UploadFileRequestSchema = z.object({
  file: z.instanceof(File),
  taskId: z.string().optional(),
})

// ============================================
// Response Schemas
// ============================================

// Note: UploadFileResponseSchema is in api.schemas.ts for backward compatibility
// Re-export it here for convenience
export { UploadFileResponseSchema } from './api.schemas.js'

// ============================================
// Path Parameter Schemas
// ============================================

export const UploadPathParamsSchema = z.object({
  uploadId: z.string().min(1),
  filename: z.string().min(1),
})

// ============================================
// Derived Types
// ============================================

export type UploadFileRequest = z.infer<typeof UploadFileRequestSchema>
export type UploadPathParams = z.infer<typeof UploadPathParamsSchema>
