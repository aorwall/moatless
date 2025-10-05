import { buildPath, type RouteParams, type RouteSuccess } from '@moatless/api-contract'
import {
  getServerStatusRoute,
  restartServerRoute,
  startServerRoute,
  stopServerRoute,
} from '@moatless/api-contract/routes/preview'
import { getWorkspaceLogsRoute } from '@moatless/api-contract/routes/workspace'
import { apiClient } from '@/lib/api/client'

type ServerStatus = RouteSuccess<typeof getServerStatusRoute>
type ServerLogsResponse = RouteSuccess<typeof getWorkspaceLogsRoute>

export const serverApi = {
  async startServer(taskId: RouteParams<typeof startServerRoute>['taskId']): Promise<ServerStatus> {
    const path = buildPath(startServerRoute, { taskId })
    return await apiClient.post<ServerStatus>(path)
  },

  async stopServer(
    taskId: RouteParams<typeof stopServerRoute>['taskId'],
  ): Promise<RouteSuccess<typeof stopServerRoute>> {
    const path = buildPath(stopServerRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof stopServerRoute>>(path)
  },

  async restartServer(
    taskId: RouteParams<typeof restartServerRoute>['taskId'],
  ): Promise<ServerStatus> {
    const path = buildPath(restartServerRoute, { taskId })
    return await apiClient.post<ServerStatus>(path)
  },

  async getServerStatus(
    taskId: RouteParams<typeof getServerStatusRoute>['taskId'],
  ): Promise<ServerStatus> {
    const path = buildPath(getServerStatusRoute, { taskId })
    return await apiClient.get<ServerStatus>(path)
  },

  async getServerLogs(
    taskId: RouteParams<typeof getWorkspaceLogsRoute>['taskId'],
  ): Promise<ServerLogsResponse> {
    const path = buildPath(getWorkspaceLogsRoute, { taskId })
    return await apiClient.get<ServerLogsResponse>(path)
  },
}
