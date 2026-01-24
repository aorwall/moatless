import { buildPath, type RouteParams, type RouteSuccess } from '@moatless/api-contract'
import { getServerStatusRoute } from '@moatless/api-contract/preview.ts'
import { getWorkspaceLogsRoute } from '@moatless/api-contract/workspace.ts'
import { apiClient } from '@/lib/api/client'

type ServerStatus = RouteSuccess<typeof getServerStatusRoute>
type ServerLogsResponse = RouteSuccess<typeof getWorkspaceLogsRoute>

export const serverApi = {
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
