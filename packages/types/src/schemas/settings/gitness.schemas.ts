import { z } from 'zod'

// ===== Gitness Service Connection Schemas =====

/**
 * Gitness configuration response
 */
export const GitnessConfigResponseSchema = z.object({
  connected: z.boolean().describe('Whether Gitness is connected'),
  login: z.string().optional().describe('Gitness username'),
  baseUrl: z.string().optional().describe('Gitness server base URL'),
})

/**
 * Request to save Personal Access Token with base URL
 */
export const GitnessPATRequestSchema = z.object({
  baseUrl: z.string().min(1).describe('Gitness server base URL (e.g. https://gitness.example.com)'),
  accessToken: z.string().min(1).describe('Gitness Personal Access Token'),
})

// Type exports
export type GitnessConfigResponse = z.infer<typeof GitnessConfigResponseSchema>
export type GitnessPATRequest = z.infer<typeof GitnessPATRequestSchema>
