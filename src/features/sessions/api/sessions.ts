import { apiClient } from '@/lib/api/client';
import type { AgentTriggerRequest, AgentTriggerResponse } from '@/types/agent';
import type { AgentEvent } from '@/types/agent-events';
import type { GitDiffResponse } from '@/types/git-diff';
import type { SessionMessages, SessionMetadata } from '@/types/session';

export const sessionsApi = {
  async getActiveSessions(): Promise<Array<string>> {
    const sessions = await apiClient.get<Array<SessionMetadata>>(
      '/api/v1/sessions',
      {
        active_only: true,
      },
    );
    return sessions.map((session) => session.session_id);
  },

  async listSessions(params?: {
    activeOnly?: boolean;
    limit?: number;
    offset?: number;
  }): Promise<Array<SessionMetadata>> {
    return await apiClient.get<Array<SessionMetadata>>('/api/v1/sessions', {
      active_only: params?.activeOnly,
      limit: params?.limit,
      offset: params?.offset,
    });
  },

  async getSession(sessionId: string): Promise<SessionMetadata> {
    return await apiClient.get<SessionMetadata>(
      `/api/v1/sessions/${sessionId}`,
    );
  },

  async getSessionMessages(
    sessionId: string,
    params?: {
      limit?: number;
      offset?: number;
    },
  ): Promise<SessionMessages> {
    return await apiClient.get<SessionMessages>(
      `/api/v1/sessions/${sessionId}/messages`,
      params,
    );
  },

  async getSessionMetadata(sessionId: string): Promise<SessionMetadata> {
    return await apiClient.get<SessionMetadata>(
      `/api/v1/sessions/${sessionId}`,
    );
  },

  async createSession(
    request: AgentTriggerRequest,
  ): Promise<AgentTriggerResponse> {
    return await apiClient.post<AgentTriggerResponse>(
      '/api/v1/sessions',
      request,
    );
  },

  async sendMessage(
    sessionId: string,
    message: string,
  ): Promise<AgentTriggerResponse> {
    return await apiClient.post<AgentTriggerResponse>(
      `/api/v1/sessions/${sessionId}/messages`,
      { message },
    );
  },

  // Deprecated: Use createSession instead
  triggerAgent(request: AgentTriggerRequest): Promise<AgentTriggerResponse> {
    return this.createSession(request);
  },

  // Deprecated: Use deleteSession instead
  async stopAgent(
    sessionId: string,
  ): Promise<{ status: string; session_id: string }> {
    return await apiClient.delete(`/api/v1/sessions/${sessionId}`);
  },

  async getSessionEvents(
    sessionId: string,
    params?: {
      limit?: number;
      offset?: number;
    },
  ): Promise<Array<AgentEvent>> {
    return await apiClient.get<Array<AgentEvent>>(
      `/api/v1/sessions/${sessionId}/events`,
      params,
    );
  },

  async getSessionGitDiff(sessionId: string): Promise<GitDiffResponse> {
    return await apiClient.get<GitDiffResponse>(
      `/api/v1/sessions/${sessionId}/git-diff`,
    );
  },
};
