import type {
  CreateProviderConnectionRequest,
  CreateTaskRouteRequest,
  CreateTaskTriggerRequest,
  ProviderConnectionResponse,
  TaskRouteResponse,
  TaskTriggerResponse,
  UpdateTaskRouteRequest,
  UpdateTaskTriggerRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createProviderConnection as createProviderConnectionHandler,
  listIntegrationProviders as listIntegrationProvidersHandler,
  listProviderConnections as listProviderConnectionsHandler,
} from '@moatless/api-client/sandbox/integrations/integrations'
import {
  createTaskRoute as createTaskRouteHandler,
  listTaskRoutes as listTaskRoutesHandler,
  updateTaskRoute as updateTaskRouteHandler,
} from '@moatless/api-client/sandbox/task-routes/task-routes'
import {
  createTaskTrigger as createTaskTriggerHandler,
  deleteTaskTrigger as deleteTaskTriggerHandler,
  listTaskTriggers as listTaskTriggersHandler,
  updateTaskTrigger as updateTaskTriggerHandler,
} from '@moatless/api-client/sandbox/triggers/triggers'

export const integrationsApi = {
  async listProviders(): Promise<string[]> {
    return await unwrapResponse<string[]>(listIntegrationProvidersHandler())
  },

  async listProviderConnections(): Promise<ProviderConnectionResponse[]> {
    return await unwrapResponse<ProviderConnectionResponse[]>(listProviderConnectionsHandler())
  },

  async createProviderConnection(
    request: CreateProviderConnectionRequest,
  ): Promise<ProviderConnectionResponse> {
    return await unwrapResponse<ProviderConnectionResponse>(
      createProviderConnectionHandler(request),
    )
  },

  async listTaskRoutes(): Promise<TaskRouteResponse[]> {
    return await unwrapResponse<TaskRouteResponse[]>(listTaskRoutesHandler())
  },

  async createTaskRoute(request: CreateTaskRouteRequest): Promise<TaskRouteResponse> {
    return await unwrapResponse<TaskRouteResponse>(createTaskRouteHandler(request))
  },

  async updateTaskRoute(id: string, request: UpdateTaskRouteRequest): Promise<TaskRouteResponse> {
    return await unwrapResponse<TaskRouteResponse>(updateTaskRouteHandler(id, request))
  },

  async listTaskTriggers(): Promise<TaskTriggerResponse[]> {
    return await unwrapResponse<TaskTriggerResponse[]>(listTaskTriggersHandler())
  },

  async createTaskTrigger(request: CreateTaskTriggerRequest): Promise<TaskTriggerResponse> {
    return await unwrapResponse<TaskTriggerResponse>(createTaskTriggerHandler(request))
  },

  async updateTaskTrigger(
    id: string,
    request: UpdateTaskTriggerRequest,
  ): Promise<TaskTriggerResponse> {
    return await unwrapResponse<TaskTriggerResponse>(updateTaskTriggerHandler(id, request))
  },

  async deleteTaskTrigger(id: string): Promise<void> {
    await unwrapResponse<void>(deleteTaskTriggerHandler(id))
  },
}
