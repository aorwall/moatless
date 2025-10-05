import { z } from 'zod'

export const pathParams = {
  taskId: z.object({ taskId: z.string().min(1) }),
} as const

export type TaskIdParams = typeof pathParams.taskId
