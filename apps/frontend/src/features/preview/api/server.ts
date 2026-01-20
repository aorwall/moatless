import { buildPath, type RouteParams, type RouteSuccess } from '@moatless/api-contract'
import {
  deleteBrowserScreenshotsRoute,
  getBrowserScreenshotsRoute,
  getServerStatusRoute,
  restartServerRoute,
  startServerRoute,
  stopServerRoute,
} from '@moatless/api-contract/preview.ts'
import { getWorkspaceLogsRoute } from '@moatless/api-contract/workspace.ts'
import { apiClient } from '@/lib/api/client'

type ServerStatus = RouteSuccess<typeof getServerStatusRoute>
type ServerLogsResponse = RouteSuccess<typeof getWorkspaceLogsRoute>
type ScreenshotListResponse = RouteSuccess<typeof getBrowserScreenshotsRoute>
type ScreenshotClearResponse = RouteSuccess<typeof deleteBrowserScreenshotsRoute>

export const serverApi = {
  async startServer(
    taskId: RouteParams<typeof startServerRoute>['taskId'],
  ): Promise<RouteSuccess<typeof startServerRoute>> {
    const path = buildPath(startServerRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof startServerRoute>>(path)
  },

  async stopServer(
    taskId: RouteParams<typeof stopServerRoute>['taskId'],
  ): Promise<RouteSuccess<typeof stopServerRoute>> {
    const path = buildPath(stopServerRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof stopServerRoute>>(path)
  },

  async restartServer(
    taskId: RouteParams<typeof restartServerRoute>['taskId'],
  ): Promise<RouteSuccess<typeof restartServerRoute>> {
    const path = buildPath(restartServerRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof restartServerRoute>>(path)
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

export const browserApi = {
  async getScreenshots(
    taskId: RouteParams<typeof getBrowserScreenshotsRoute>['taskId'],
  ): Promise<ScreenshotListResponse> {
    const path = buildPath(getBrowserScreenshotsRoute, { taskId })
    return await apiClient.get<ScreenshotListResponse>(path)
  },

  getScreenshotUrl(taskId: string, filename: string): string {
    return `/api/v1/tasks/${taskId}/browser/screenshots/${filename}`
  },

  async clearScreenshots(
    taskId: RouteParams<typeof deleteBrowserScreenshotsRoute>['taskId'],
  ): Promise<ScreenshotClearResponse> {
    const path = buildPath(deleteBrowserScreenshotsRoute, { taskId })
    return await apiClient.delete<ScreenshotClearResponse>(path)
  },
}
