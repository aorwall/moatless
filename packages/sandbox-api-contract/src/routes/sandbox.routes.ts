import { createRoute } from '@hono/zod-openapi'
import {
  SandboxProvisionRequestSchema,
  SandboxProvisionResponseSchema,
  SandboxStatusResponseSchema,
} from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const sandboxProvisionRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.SANDBOX_PROVISION,
  tags: ['Sandbox'],
  description: 'Provision a sandbox (git clone, setup)',
  request: {
    body: {
      content: {
        'application/json': {
          schema: SandboxProvisionRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Sandbox provisioned',
      content: {
        'application/json': {
          schema: SandboxProvisionResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to provision sandbox',
      content: {
        'application/json': {
          schema: SandboxProvisionResponseSchema,
        },
      },
    },
  },
})

export const sandboxStatusRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.SANDBOX_STATUS,
  tags: ['Sandbox'],
  description: 'Get sandbox status',
  responses: {
    200: {
      description: 'Sandbox status',
      content: {
        'application/json': {
          schema: SandboxStatusResponseSchema,
        },
      },
    },
  },
})
