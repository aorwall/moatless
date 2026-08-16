import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  deleteGitnessConfig,
  getGitnessConfig,
  saveGitnessPat,
} from '@moatless/api-client/sandbox/git-host-access/git-host-access'
import type {
  GitnessProviderTokenStatusResponse,
  SaveGitnessProviderTokenRequest,
} from '@moatless/api-client/sandbox/model'

export const gitnessConnectionApi = {
  getConfig: (): Promise<GitnessProviderTokenStatusResponse> =>
    unwrapResponse<GitnessProviderTokenStatusResponse>(getGitnessConfig()),

  disconnect: (): Promise<GitnessProviderTokenStatusResponse> =>
    unwrapResponse<GitnessProviderTokenStatusResponse>(deleteGitnessConfig()),

  savePAT: (
    request: SaveGitnessProviderTokenRequest,
  ): Promise<GitnessProviderTokenStatusResponse> =>
    unwrapResponse<GitnessProviderTokenStatusResponse>(saveGitnessPat(request)),
}
