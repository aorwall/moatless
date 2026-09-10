import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  deleteForgejoConfig,
  deleteForgejoPatOverride,
  getForgejoConfig,
  listForgejoConnections,
  saveForgejoPat,
} from '@moatless/api-client/sandbox/git-host-access/git-host-access'
import type {
  ForgejoConnectionsResponse,
  ForgejoProviderTokenStatusResponse,
  SaveForgejoProviderTokenRequest,
} from '@moatless/api-client/sandbox/model'

/**
 * Every call names the instance: Forgejo is self-hostable, so one person can
 * hold a separate credential per host.
 */
export const forgejoConnectionApi = {
  /** Every registered instance, each with the caller's own connection to it. */
  listInstances: (): Promise<ForgejoConnectionsResponse> =>
    unwrapResponse<ForgejoConnectionsResponse>(listForgejoConnections()),

  getConfig: (host: string): Promise<ForgejoProviderTokenStatusResponse> =>
    unwrapResponse<ForgejoProviderTokenStatusResponse>(getForgejoConfig({ host })),

  disconnect: (host: string): Promise<ForgejoProviderTokenStatusResponse> =>
    unwrapResponse<ForgejoProviderTokenStatusResponse>(deleteForgejoConfig({ host })),

  savePAT: (
    request: SaveForgejoProviderTokenRequest,
  ): Promise<ForgejoProviderTokenStatusResponse> =>
    unwrapResponse<ForgejoProviderTokenStatusResponse>(saveForgejoPat(request)),

  /** Removes the token standing in front of an OAuth credential, restoring it. */
  removePatOverride: (host: string): Promise<ForgejoProviderTokenStatusResponse> =>
    unwrapResponse<ForgejoProviderTokenStatusResponse>(deleteForgejoPatOverride({ host })),
}
