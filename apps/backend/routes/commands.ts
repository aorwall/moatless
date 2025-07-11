import { CommandNotFoundError } from '@moatless/shared/constants/errors'
import { CreateCommandSchema, UpdateCommandSchema } from '@moatless/shared/schemas/command.schemas'
import type { CommandList } from '@moatless/shared/types/command'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'

// Response schemas
const commandResponseSchema = z.object({
  name: z.string(),
  description: z.string(),
  prompt: z.string(),
  conditions: z.any().optional(), // CommandConditionsSchema
  icon: z.string().optional(),
  variant: z.enum(['default', 'outline', 'secondary', 'ghost', 'destructive']).optional(),
  active: z.boolean().optional(),
  willNotResume: z.boolean().optional(),
})

const commandListResponseSchema = z.object({
  commands: z.array(commandResponseSchema),
})

const deleteResponseSchema = z.object({
  status: z.literal('deleted'),
})

export async function registerCommandRoutes(app: Hono<AppBindings>, prefix: string) {
  // Create command
  app.post(
    prefix,
    describeRoute({
      description: 'Create a new command',
      requestBody: {
        description: 'Command data to create',
        content: {
          'application/json': {
            schema: resolver(CreateCommandSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Command created successfully',
          content: {
            'application/json': {
              schema: resolver(commandResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { command: commandService } = services

      const body = await c.req.json()
      const validatedBody = CreateCommandSchema.parse(body)

      const command = await commandService.createCommand(validatedBody)
      return c.json(command)
    },
  )

  // List commands
  app.get(
    prefix,
    describeRoute({
      description: 'Get list of all commands',
      responses: {
        200: {
          description: 'List of commands',
          content: {
            'application/json': {
              schema: resolver(commandListResponseSchema),
            },
          },
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { command: commandService } = services

      const commands = await commandService.listCommands()
      return c.json({ commands } as CommandList)
    },
  )

  // Get command by name
  app.get(
    `${prefix}/:commandName`,
    describeRoute({
      description: 'Get a specific command by name',
      parameters: [
        {
          name: 'commandName',
          in: 'path',
          required: true,
          schema: {
            type: 'string',
          },
          description: 'Name of the command to retrieve',
        },
      ],
      responses: {
        200: {
          description: 'Command details',
          content: {
            'application/json': {
              schema: resolver(commandResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid command name',
        },
        404: {
          description: 'Command not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { command: commandService } = services

      const commandName = c.req.param('commandName')
      if (!commandName) {
        throw new HTTPException(400, { message: 'Command name is required' })
      }

      const command = await commandService.getCommand(commandName)

      if (!command) {
        throw new CommandNotFoundError(commandName)
      }

      return c.json(command)
    },
  )

  // Update command
  app.put(
    `${prefix}/:commandName`,
    describeRoute({
      description: 'Update an existing command',
      parameters: [
        {
          name: 'commandName',
          in: 'path',
          required: true,
          schema: {
            type: 'string',
          },
          description: 'Name of the command to update',
        },
      ],
      requestBody: {
        description: 'Updated command data',
        content: {
          'application/json': {
            schema: resolver(UpdateCommandSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Command updated successfully',
          content: {
            'application/json': {
              schema: resolver(commandResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request body or command name',
        },
        404: {
          description: 'Command not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { command: commandService } = services

      const commandName = c.req.param('commandName')
      if (!commandName) {
        throw new HTTPException(400, { message: 'Command name is required' })
      }

      const body = await c.req.json()
      const validatedBody = UpdateCommandSchema.parse(body)

      const updatedCommand = await commandService.updateCommand(commandName, validatedBody)
      return c.json(updatedCommand)
    },
  )

  // Delete command
  app.delete(
    `${prefix}/:commandName`,
    describeRoute({
      description: 'Delete a command',
      parameters: [
        {
          name: 'commandName',
          in: 'path',
          required: true,
          schema: {
            type: 'string',
          },
          description: 'Name of the command to delete',
        },
      ],
      responses: {
        200: {
          description: 'Command deleted successfully',
          content: {
            'application/json': {
              schema: resolver(deleteResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid command name',
        },
        404: {
          description: 'Command not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { command: commandService } = services

      const commandName = c.req.param('commandName')
      if (!commandName) {
        throw new HTTPException(400, { message: 'Command name is required' })
      }

      await commandService.deleteCommand(commandName)
      return c.json({ status: 'deleted' })
    },
  )
}
