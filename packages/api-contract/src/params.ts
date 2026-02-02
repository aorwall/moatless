import { z } from 'zod'

export const pathParams = {
  taskId: z.object({ taskId: z.string().min(1) }),
  repositoryId: z.object({ repositoryId: z.string().min(1) }),
  commandName: z.object({ commandName: z.string().min(1) }),
  uploadId: z.object({ uploadId: z.string().min(1) }),
  filename: z.object({ filename: z.string().min(1) }),
  sessionId: z.object({ sessionId: z.string().min(1) }),
  uploadIdAndFilename: z.object({
    uploadId: z.string().min(1),
    filename: z.string().min(1),
  }),
  taskIdAndFilename: z.object({
    taskId: z.string().min(1),
    filename: z.string().min(1),
  }),
  taskIdAndMessageId: z.object({
    taskId: z.string().min(1),
    messageId: z.string().min(1),
  }),
} as const
