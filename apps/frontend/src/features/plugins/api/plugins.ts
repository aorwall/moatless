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
  listPluginActivations as listPluginActivationsHandler,
  listPluginSkills as listPluginSkillsHandler,
  listPlugins as listPluginsHandler,
  setPluginActivation as setPluginActivationHandler,
} from '@moatless/api-client/sandbox/plugins/plugins'

export const pluginsApi = {
  async listPlugins(): Promise<PluginResponse[]> {
    return await unwrapResponse(listPluginsHandler())
  },

  async listPluginSkills(pluginId: string): Promise<PluginSkillResponse[]> {
    return await unwrapResponse(listPluginSkillsHandler(pluginId))
  },

  /**
   * The records that bear on this user: the global ones plus their own, plus
   * the named workspace's when there is one.
   */
  async listPluginActivations(
    pluginId: string,
    workspaceId?: string,
  ): Promise<ActivationResponse[]> {
    return await unwrapResponse(
      listPluginActivationsHandler(pluginId, workspaceId === undefined ? {} : { workspaceId }),
    )
  },

  async createPlugin(request: CreatePluginRequest): Promise<PluginResponse> {
    return await unwrapResponse(createPluginHandler(request))
  },

  async deletePlugin(pluginId: string): Promise<void> {
    await unwrapResponse(deletePluginHandler(pluginId))
  },

  /**
   * What the calling user actually receives, once the global, personal and
   * workspace records resolve against each other.
   */
  async listEffectivePlugins(workspaceId?: string): Promise<EffectivePluginResponse[]> {
    return await unwrapResponse(
      listEffectivePluginsHandler(workspaceId === undefined ? {} : { workspaceId }),
    )
  },

  async setActivation(
    pluginId: string,
    request: SetActivationRequest,
  ): Promise<ActivationResponse> {
    return await unwrapResponse(setPluginActivationHandler(pluginId, request))
  },

  /** Not the same as turning it off — this hands the subject back to the next reach down. */
  async clearActivation(pluginId: string, params: ClearPluginActivationParams): Promise<void> {
    await unwrapResponse(clearPluginActivationHandler(pluginId, params))
  },
}
