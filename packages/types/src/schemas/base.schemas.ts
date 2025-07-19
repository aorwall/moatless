import { z } from 'zod'

// Base schemas that are used across multiple files
// This file should not import from other schema files to avoid circular dependencies

export const TaskStatusSchema = z.enum(['open', 'running', 'error', 'merged', 'closed'])

export type TaskStatus = z.infer<typeof TaskStatusSchema>
