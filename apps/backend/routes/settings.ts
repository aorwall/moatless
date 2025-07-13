import { zValidator } from '@hono/zod-validator'
import type { UpdateSettingsRequest } from '@moatless/types'
import { updateSettingsSchema } from '@moatless/types/schemas'
import { Hono } from 'hono'
import { getSettingsService } from '../services/index.js'
import { logger } from '../utils/logger.js'

const settingsRoute = new Hono()

// GET /api/settings - Get current settings and defaults
settingsRoute.get('/', async (c) => {
  try {
    const settingsService = getSettingsService()
    const settings = await settingsService.getSettings()
    return c.json(settings)
  } catch (error) {
    logger.error('Failed to get settings', error as Error)
    return c.json({ error: 'Failed to get settings' }, 500)
  }
})

// PUT /api/settings - Update settings
settingsRoute.put('/', zValidator('json', updateSettingsSchema), async (c) => {
  try {
    const settingsService = getSettingsService()
    const update = await c.req.valid('json')
    const settings = await settingsService.updateSettings(update as UpdateSettingsRequest)
    return c.json(settings)
  } catch (error) {
    logger.error('Failed to update settings', error as Error)
    return c.json({ error: 'Failed to update settings' }, 500)
  }
})

// DELETE /api/settings - Reset settings to defaults
settingsRoute.delete('/', async (c) => {
  try {
    const settingsService = getSettingsService()
    const settings = await settingsService.resetSettings()
    return c.json(settings)
  } catch (error) {
    logger.error('Failed to reset settings', error as Error)
    return c.json({ error: 'Failed to reset settings' }, 500)
  }
})

export { settingsRoute }
