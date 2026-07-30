import type {
  ActivateLoopRequest,
  CreateLoopRequest,
  Loop,
  LoopRecentTaskItem,
  ResetGitLoopResponse,
  UpdateLoopRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  activateLoop as activateLoopHandler,
  createLoop as createLoopHandler,
  deleteLoop as deleteLoopHandler,
  getLoop as getLoopHandler,
  listLoopRecentTasks as listLoopRecentTasksHandler,
  listLoops as listLoopsHandler,
  overrideLoop as overrideLoopHandler,
  pauseLoop as pauseLoopHandler,
  resetGitLoop as resetGitLoopHandler,
  resumeLoop as resumeLoopHandler,
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

  async activateLoop(id: string, request: ActivateLoopRequest): Promise<Loop> {
    return await unwrapResponse<Loop>(activateLoopHandler(id, request))
  },

  async pauseLoop(id: string): Promise<Loop> {
    return await unwrapResponse<Loop>(pauseLoopHandler(id))
  },

  async resumeLoop(id: string): Promise<Loop> {
    return await unwrapResponse<Loop>(resumeLoopHandler(id))
  },

  async overrideLoop(id: string): Promise<Loop> {
    return await unwrapResponse<Loop>(overrideLoopHandler(id))
  },

  async resetGitLoop(id: string): Promise<ResetGitLoopResponse> {
    return await unwrapResponse<ResetGitLoopResponse>(resetGitLoopHandler(id))
  },

  async listLoopRecentTasks(
    id: string,
    params?: { limit?: number },
  ): Promise<LoopRecentTaskItem[]> {
    return await unwrapResponse<LoopRecentTaskItem[]>(listLoopRecentTasksHandler(id, params))
  },
}
