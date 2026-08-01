import type {
  AdapterAppSummary,
  AdapterAppsResponse,
  SaveAdapterAppRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  adminDeleteAdapterApp,
  adminListAdapterApps,
  adminSaveAdapterApp,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'

export const adapterAppsApi = {
  async list(): Promise<AdapterAppSummary[]> {
    const response = await unwrapResponse<AdapterAppsResponse>(adminListAdapterApps())
    return response.apps
  },

  async save(
    adapterKind: string,
    appKey: string,
    request: SaveAdapterAppRequest,
  ): Promise<AdapterAppSummary> {
    return await unwrapResponse<AdapterAppSummary>(
      adminSaveAdapterApp(adapterKind, appKey, request),
    )
  },

  async delete(adapterKind: string, appKey: string): Promise<void> {
    await unwrapResponse<null>(adminDeleteAdapterApp(adapterKind, appKey))
  },
}
