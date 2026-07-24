import type {
  CreateLoopRequest,
  Loop,
  LoopRecentTaskItem,
  UpdateLoopRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createLoop as createLoopHandler,
  deleteLoop as deleteLoopHandler,
  getLoop as getLoopHandler,
  listLoopRecentTasks as listLoopRecentTasksHandler,
  listLoops as listLoopsHandler,
  updateLoop as updateLoopHandler,
} from '@moatless/api-client/sandbox/loops/loops'

export const loopsApi = {
  async listLoops(): Promise<Loop[]> {
    return await unwrapResponse<Loop[]>(listLoopsHandler())
  },

  async createLoop(request: CreateLoopRequest): Promise<Loop> {
    return await unwrapResponse<Loop>(createLoopHandler(request))
  },

  async updateLoop(id: string, request: UpdateLoopRequest): Promise<Loop> {
    return await unwrapResponse<Loop>(updateLoopHandler(id, request))
  },

  async deleteLoop(id: string): Promise<void> {
    await unwrapResponse<void>(deleteLoopHandler(id))
  },

  async getLoop(id: string): Promise<Loop> {
    return await unwrapResponse<Loop>(getLoopHandler(id))
  },

  async listLoopRecentTasks(
    id: string,
    params?: { limit?: number },
  ): Promise<LoopRecentTaskItem[]> {
    return await unwrapResponse<LoopRecentTaskItem[]>(listLoopRecentTasksHandler(id, params))
  },
}
