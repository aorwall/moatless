import {
  type GitHubSettingsResponse,
  type UpdateGitHubSettingsRequest,
  updateGitHubSettingsSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import type { AppBindings } from '../../app.js'
import { handleServiceError } from '../../utils/error-handler.js'
import { validateRequestBody } from '../../utils/route-helpers.js'

export function registerGitHubSettingsRoutes(app: Hono<AppBindings>) {
  // GET settings/github - Get GitHub settings
  app.get('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const settings: GitHubSettingsResponse = await settingsService.getGitHubSettingsResponse()
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Get GitHub settings')
    }
  })

  // PUT settings/github - Update GitHub settings
  app.put('/', async (c) => {
    try {
      const services = c.get('services')
      const { settings: settingsService } = services
      const update = await validateRequestBody(c, updateGitHubSettingsSchema)
      const settings: GitHubSettingsResponse = await settingsService.updateGitHubSettingsSection(
        update as UpdateGitHubSettingsRequest,
      )
      return c.json(settings)
    } catch (error) {
      handleServiceError(error, 'Update GitHub settings')
    }
  })
}
