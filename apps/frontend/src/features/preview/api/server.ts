import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { serverStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
import type { ServerStatus } from '@moatless/types'

export const serverApi = {
  async getServerStatus(taskId: string): Promise<ServerStatus> {
    return await unwrapResponse<ServerStatus>(serverStatusHandler(taskId))
  },
}
