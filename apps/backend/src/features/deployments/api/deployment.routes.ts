import type { OpenAPIHono } from '@hono/zod-openapi'
import { deployRoute } from '@moatless/api-contract/tasks'
import { TaskNotFoundError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app.js'
import { appContainer } from '../../../container/container.js'
import {
  DeploymentConfigurationError,
  DeploymentService,
  DeploymentWorkspaceError,
} from '../services/deployment.service.js'

/**
 * Register deployment routes.
 * Provides endpoints for deploying task workspaces to hosting providers.
 */
export function registerDeploymentRoutes(app: OpenAPIHono<AppBindings>) {
  // Deploy task to Netlify
  app.openapi(deployRoute, async (c) => {
    const deploymentService = appContainer.get(DeploymentService)
    const { taskId } = c.req.valid('param')

    try {
      const result = await deploymentService.deployTask(taskId)
      return c.json(result)
    } catch (error) {
      if (error instanceof TaskNotFoundError) {
        throw new HTTPException(404, { message: error.message })
      }
      if (error instanceof DeploymentConfigurationError) {
        throw new HTTPException(400, { message: error.message })
      }
      if (error instanceof DeploymentWorkspaceError) {
        throw new HTTPException(400, { message: error.message })
      }
      throw error
    }
  })
}
