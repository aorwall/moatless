import { createRoute } from '@hono/zod-openapi'
import {
  WorkspaceProvisionRequestSchema,
  WorkspaceProvisionResponseSchema,
  WorkspaceStatusResponseSchema,
} from '@moatless/sandbox-schemas'
import { SANDBOX_ENDPOINTS } from '../endpoints'

export const workspaceProvisionRoute = createRoute({
  method: 'post',
  path: SANDBOX_ENDPOINTS.WORKSPACE_PROVISION,
  tags: ['Workspace'],
  description: 'Provision a workspace in the sandbox (git clone, setup)',
  request: {
    body: {
      content: {
        'application/json': {
          schema: WorkspaceProvisionRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Workspace provisioned',
      content: {
        'application/json': {
          schema: WorkspaceProvisionResponseSchema,
        },
      },
    },
    500: {
      description: 'Failed to provision workspace',
      content: {
        'application/json': {
          schema: WorkspaceProvisionResponseSchema,
        },
      },
    },
  },
})

export const workspaceStatusRoute = createRoute({
  method: 'get',
  path: SANDBOX_ENDPOINTS.WORKSPACE_STATUS,
  tags: ['Workspace'],
  description: 'Get workspace status',
  responses: {
    200: {
      description: 'Workspace status',
      content: {
        'application/json': {
          schema: WorkspaceStatusResponseSchema,
        },
      },
    },
  },
})
