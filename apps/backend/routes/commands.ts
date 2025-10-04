import type { CommandList } from '@moatless/types'
import { CommandNotFoundError, CreateCommandSchema, UpdateCommandSchema } from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
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

const _commandListResponseSchema = z.object({
  commands: z.array(commandResponseSchema),
})

const _deleteResponseSchema = z.object({
  status: z.literal('deleted'),
})

export async function registerCommandRoutes(app: Hono<AppBindings>, prefix: string) {
  // Create command
  app.post(prefix, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services

    const body = await c.req.json()
    const validatedBody = CreateCommandSchema.parse(body)

    const command = await commandService.createCommand(validatedBody)
    return c.json(command)
  })

  // List commands
  app.get(prefix, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services

    const commands = await commandService.listCommands()
    return c.json({ commands } as CommandList)
  })

  // Get command by name
  app.get(`${prefix}/:commandName`, async (c) => {
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
  })

  // Update command
  app.put(`${prefix}/:commandName`, async (c) => {
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
  })

  // Delete command
  app.delete(`${prefix}/:commandName`, async (c) => {
    const services = c.get('services')
    const { command: commandService } = services

    const commandName = c.req.param('commandName')
    if (!commandName) {
      throw new HTTPException(400, { message: 'Command name is required' })
    }

    await commandService.deleteCommand(commandName)
    return c.json({ status: 'deleted' })
  })
}
