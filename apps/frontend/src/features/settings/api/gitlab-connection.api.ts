import type { GitLabConfigResponse, GitLabPATRequest } from '@moatless/types'

type DisconnectGitLabResponse = { success: boolean; message: string }
type SaveGitLabPATResponse = { success: boolean; message: string }

// TODO: GitLab endpoints not implemented in Rust backend
export const gitlabConnectionApi = {
  getConfig: (): Promise<GitLabConfigResponse> => {
    return Promise.resolve({ connected: false, oauthEnabled: false } as GitLabConfigResponse)
  },

  disconnect: (): Promise<DisconnectGitLabResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },

  savePAT: (_request: GitLabPATRequest): Promise<SaveGitLabPATResponse> => {
    return Promise.resolve({ success: false, message: 'Not implemented' })
  },
}
