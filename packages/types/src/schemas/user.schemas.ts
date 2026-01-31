import { z } from 'zod'

export const UserSummarySchema = z.object({
  id: z.string(),
  login: z.string(),
  name: z.string().nullable(),
  avatarUrl: z.string().nullable(),
})

export const UserListResponseSchema = z.object({
  users: z.array(UserSummarySchema),
})

export type UserSummary = z.infer<typeof UserSummarySchema>
export type UserListResponse = z.infer<typeof UserListResponseSchema>
