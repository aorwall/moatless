import type { OpenAPIHono } from '@hono/zod-openapi'
import { createRoute } from '@hono/zod-openapi'
import type { CommandList } from '@moatless/types'
import {
  CommandListResponseSchema,
  CommandNotFoundError,
  CommandSchema,
  CreateCommandSchema,
  UpdateCommandSchema,
} from '@moatless/types'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import {
  createDeleteRoute,
  createGetByIdRoute,
  createGetListRoute,
  createPostRoute,
  jsonBody,
  pathParams,
  withStandardErrors,
} from '../utils/openapi-helpers.js'

export async function registerCommandRoutes(app: OpenAPIHono<AppBindings>) {
  // Create command
  const createCommandRoute = createPostRoute(
    '/api/v1/commands',
    CreateCommandSchema,
    CommandSchema,
    {
      operation: 'Create a new custom command for task automation and workflow execution',
      request: 'Command configuration including name, shell script, and execution parameters',
      response: 'Returns created command with unique name and configuration details',
    },
    201,
    ['Commands'],
  )

  app.openapi(createCommandRoute, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services
    const validatedBody = c.req.valid('json')

    const command = await commandService.createCommand(validatedBody)
    return c.json(command)
  })

  // List commands
  const listCommandsRoute = createGetListRoute(
    '/api/v1/commands',
    z.object({}),
    CommandListResponseSchema,
    {
      operation: 'List all configured custom commands available for task execution',
      response: 'Returns array of commands with names, scripts, and configuration',
    },
    ['Commands'],
  )

  app.openapi(listCommandsRoute, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services

    const commands = await commandService.listCommands()
    return c.json({ commands } as CommandList)
  })

  // Get command by name
  const getCommandRoute = createGetByIdRoute(
    '/api/v1/commands/{commandName}',
    pathParams.commandName,
    CommandSchema,
    {
      operation: 'Retrieve a specific custom command by its unique name',
      response: 'Returns command configuration including script, parameters, and metadata',
    },
    ['Commands'],
  )

  app.openapi(getCommandRoute, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services
    const { commandName } = c.req.valid('param')

    const command = await commandService.getCommand(commandName)
    if (!command) {
      throw new CommandNotFoundError(commandName)
    }

    return c.json(command)
  })

  // Update command
  const updateCommandRoute = createRoute({
    method: 'put',
    path: '/api/v1/commands/{commandName}',
    tags: ['Commands'],
    description: 'Update an existing custom command configuration',
    request: {
      params: pathParams.commandName,
      ...jsonBody(
        UpdateCommandSchema,
        'Updated command configuration including script and parameters',
      ),
    },
    responses: withStandardErrors(
      CommandSchema,
      'Returns updated command with modified configuration',
      200,
    ),
  })

  app.openapi(updateCommandRoute, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services
    const { commandName } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const updatedCommand = await commandService.updateCommand(commandName, validatedBody)
    return c.json(updatedCommand)
  })

  // Delete command
  const deleteCommandRoute = createDeleteRoute(
    '/api/v1/commands/{commandName}',
    pathParams.commandName,
    'Command deleted successfully',
    ['Commands'],
  )

  app.openapi(deleteCommandRoute, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services
    const { commandName } = c.req.valid('param')

    await commandService.deleteCommand(commandName)
    return c.json({ status: 'deleted' })
  })
}
