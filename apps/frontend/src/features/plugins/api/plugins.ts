import type { CreatePluginRequest, PluginResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createPlugin as createPluginHandler,
  deletePlugin as deletePluginHandler,
  listPlugins as listPluginsHandler,
  listRepoPlugins as listRepoPluginsHandler,
} from '@moatless/api-client/sandbox/plugins/plugins'

export const pluginsApi = {
  async listPlugins(): Promise<PluginResponse[]> {
    return await unwrapResponse(listPluginsHandler())
  },

  async listRepoPlugins(repoId: string): Promise<PluginResponse[]> {
    return await unwrapResponse(listRepoPluginsHandler(repoId))
  },

  async createPlugin(request: CreatePluginRequest): Promise<PluginResponse> {
    return await unwrapResponse(createPluginHandler(request))
  },

  async deletePlugin(pluginId: string): Promise<void> {
    await unwrapResponse(deletePluginHandler(pluginId))
  },
}
