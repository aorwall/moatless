import { z } from 'zod'

/**
 * Browser action types for screenshots
 */
export const BrowserActionSchema = z.enum(['navigate', 'click', 'fill', 'manual'])
export type BrowserAction = z.infer<typeof BrowserActionSchema>

/**
 * Metadata for a captured browser screenshot
 */
export const ScreenshotMetadataSchema = z.object({
  id: z.string(),
  filename: z.string(),
  timestamp: z.string(),
  action: BrowserActionSchema,
  pageName: z.string(),
  url: z.string(),
  width: z.number(),
  height: z.number(),
})
export type ScreenshotMetadata = z.infer<typeof ScreenshotMetadataSchema>

/**
 * Response for listing screenshots
 */
export const ScreenshotListResponseSchema = z.object({
  screenshots: z.array(ScreenshotMetadataSchema),
  total: z.number(),
})
export type ScreenshotListResponse = z.infer<typeof ScreenshotListResponseSchema>

/**
 * Response for clearing screenshots
 */
export const ScreenshotClearResponseSchema = z.object({
  success: z.boolean(),
  deleted: z.number(),
})
export type ScreenshotClearResponse = z.infer<typeof ScreenshotClearResponseSchema>
