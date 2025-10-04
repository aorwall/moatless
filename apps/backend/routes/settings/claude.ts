import {
  type ClaudeSettingsResponse,
  type UpdateClaudeSettingsRequest,
  updateClaudeSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerClaudeSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/claude - Get Claude settings
  app.get('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const settings: ClaudeSettingsResponse = await settingsService.getClaudeSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get Claude settings')
    }
  })

  // PUT settings/claude - Update Claude settings
  app.put('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const update = await validateRequestBody(c, updateClaudeSettingsSchema)
      const settings: ClaudeSettingsResponse = await settingsService.updateClaudeSettingsSection(
        update as UpdateClaudeSettingsRequest,
      )
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update Claude settings')
    }
  })
}
