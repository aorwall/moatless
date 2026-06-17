import type { ListTaskConnectionsParams, TaskConnectionsResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { listTaskConnections as listTaskConnectionsHandler } from '@moatless/api-client/sandbox/tasks/tasks'

export const taskConnectionsApi = {
  async list(params?: ListTaskConnectionsParams): Promise<TaskConnectionsResponse> {
    return await unwrapResponse<TaskConnectionsResponse>(listTaskConnectionsHandler(params))
  },
}
