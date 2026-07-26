import type {
  ActivationResponse,
  ClearPluginActivationParams,
  CreatePluginRequest,
  EffectivePluginResponse,
  PluginResponse,
  PluginSkillResponse,
  SetActivationRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  clearPluginActivation as clearPluginActivationHandler,
  createPlugin as createPluginHandler,
  deletePlugin as deletePluginHandler,
  listEffectivePlugins as listEffectivePluginsHandler,
  listPluginSkills as listPluginSkillsHandler,
  listPlugins as listPluginsHandler,
  listRepoPlugins as listRepoPluginsHandler,
  setPluginActivation as setPluginActivationHandler,
} from '@moatless/api-client/sandbox/plugins/plugins'

export const pluginsApi = {
  async listPlugins(): Promise<PluginResponse[]> {
    return await unwrapResponse(listPluginsHandler())
  },

  async listRepoPlugins(repoId: string): Promise<PluginResponse[]> {
    return await unwrapResponse(listRepoPluginsHandler(repoId))
  },

  async listPluginSkills(pluginId: string): Promise<PluginSkillResponse[]> {
    return await unwrapResponse(listPluginSkillsHandler(pluginId))
  },

  async createPlugin(request: CreatePluginRequest): Promise<PluginResponse> {
    return await unwrapResponse(createPluginHandler(request))
  },

  async deletePlugin(pluginId: string): Promise<void> {
    await unwrapResponse(deletePluginHandler(pluginId))
  },

  /** What the calling user actually receives, once global and personal records resolve. */
  async listEffectivePlugins(): Promise<EffectivePluginResponse[]> {
    return await unwrapResponse(listEffectivePluginsHandler())
  },

  async setActivation(
    pluginId: string,
    request: SetActivationRequest,
  ): Promise<ActivationResponse> {
    return await unwrapResponse(setPluginActivationHandler(pluginId, request))
  },

  /** Not the same as turning it off — this hands the user back to the global default. */
  async clearActivation(pluginId: string, params: ClearPluginActivationParams): Promise<void> {
    await unwrapResponse(clearPluginActivationHandler(pluginId, params))
  },
}
