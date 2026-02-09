import type { ServerStatusResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { serverStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'

export const serverApi = {
  async getServerStatus(taskId: string): Promise<ServerStatusResponse> {
    return await unwrapResponse(serverStatusHandler(taskId))
  },
}
