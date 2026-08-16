import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  deleteCodexConfig,
  getCodexConfig,
  pollCodexDeviceLogin,
  saveCodexAuth,
  startCodexDeviceLogin,
} from '@moatless/api-client/sandbox/agent-harness-access/agent-harness-access'
import type {
  CodexAgentHarnessCredentialStatusResponse,
  CodexDeviceCodePollRequest,
  CodexDeviceCodePollResponse,
  CodexDeviceCodeStartResponse,
  SaveCodexAgentHarnessCredentialRequest,
} from '@moatless/api-client/sandbox/model'

export const codexConnectionApi = {
  getConfig: (): Promise<CodexAgentHarnessCredentialStatusResponse> =>
    unwrapResponse<CodexAgentHarnessCredentialStatusResponse>(getCodexConfig()),

  saveAuthJson: (
    request: SaveCodexAgentHarnessCredentialRequest,
  ): Promise<CodexAgentHarnessCredentialStatusResponse> =>
    unwrapResponse<CodexAgentHarnessCredentialStatusResponse>(saveCodexAuth(request)),

  disconnect: (): Promise<CodexAgentHarnessCredentialStatusResponse> =>
    unwrapResponse<CodexAgentHarnessCredentialStatusResponse>(deleteCodexConfig()),

  startDeviceLogin: (): Promise<CodexDeviceCodeStartResponse> =>
    unwrapResponse<CodexDeviceCodeStartResponse>(startCodexDeviceLogin()),

  pollDeviceLogin: (request: CodexDeviceCodePollRequest): Promise<CodexDeviceCodePollResponse> =>
    unwrapResponse<CodexDeviceCodePollResponse>(pollCodexDeviceLogin(request)),
}
