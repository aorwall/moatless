import type {
  UseMutationOptions,
  UseQueryOptions,
} from '@tanstack/react-query';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import { sessionsApi } from '@/features/sessions/api/sessions';
import { queryKeys } from '@/lib/api/query-keys';
import type { AgentTriggerRequest, AgentTriggerResponse } from '@/types/agent';
import type { AgentEvent } from '@/types/agent-events';
import type { GitDiffResponse } from '@/types/git-diff';
import type { SessionMessages, SessionMetadata } from '@/types/session';

// Query hooks
export function useSessionsList(
  params?: {
    activeOnly?: boolean;
    limit?: number;
    offset?: number;
  },
  options?: Omit<
    UseQueryOptions<Array<SessionMetadata>>,
    'queryKey' | 'queryFn'
  >,
) {
  return useQuery({
    queryKey: queryKeys.sessionsList(params),
    queryFn: () => sessionsApi.listSessions(params),
    ...options,
  });
}

export function useSession(
  sessionId: string,
  options?: Omit<UseQueryOptions<SessionMetadata>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.sessionDetail(sessionId),
    queryFn: () => sessionsApi.getSession(sessionId),
    enabled: !!sessionId,
    ...options,
  });
}

export function useSessionMessages(
  sessionId: string,
  params?: {
    limit?: number;
    offset?: number;
  },
  options?: Omit<UseQueryOptions<SessionMessages>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.sessionMessages(sessionId, params),
    queryFn: () => sessionsApi.getSessionMessages(sessionId, params),
    enabled: !!sessionId,
    ...options,
  });
}

export function useActiveSessions(
  options?: Omit<UseQueryOptions<Array<string>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.sessionsList({ activeOnly: true }),
    queryFn: () => sessionsApi.getActiveSessions(),
    ...options,
  });
}

// Mutation hooks
export function useCreateSession(
  options?: UseMutationOptions<
    AgentTriggerResponse,
    Error,
    AgentTriggerRequest
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: AgentTriggerRequest) =>
      sessionsApi.createSession(request),
    onSuccess: (data, variables, context) => {
      // Invalidate sessions list to refresh with new session
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions() });
      options?.onSuccess?.(data, variables, context);
    },
    ...options,
  });
}

export function useSendMessage(
  options?: UseMutationOptions<
    AgentTriggerResponse,
    Error,
    { sessionId: string; message: string }
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ sessionId, message }) =>
      sessionsApi.sendMessage(sessionId, message),
    onSuccess: (data, variables, context) => {
      // Invalidate messages for this session
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionMessages(variables.sessionId),
      });
      // Invalidate session detail to update metadata
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionDetail(variables.sessionId),
      });
      options?.onSuccess?.(data, variables, context);
    },
    ...options,
  });
}

export function useStopAgent(
  options?: UseMutationOptions<
    { status: string; session_id: string },
    Error,
    string
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (sessionId: string) => sessionsApi.stopAgent(sessionId),
    onSuccess: (data, sessionId, context) => {
      // Invalidate all session-related queries
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions() });
      options?.onSuccess?.(data, sessionId, context);
    },
    ...options,
  });
}

export function useSessionEvents(
  sessionId: string,
  params?: {
    limit?: number;
    offset?: number;
  },
  options?: Omit<UseQueryOptions<Array<AgentEvent>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.sessionEvents(sessionId, params),
    queryFn: () => sessionsApi.getSessionEvents(sessionId, params),
    enabled: !!sessionId,
    ...options,
  });
}

export function useSessionGitDiff(
  sessionId: string,
  options?: Omit<UseQueryOptions<GitDiffResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.sessionGitDiff(sessionId),
    queryFn: () => sessionsApi.getSessionGitDiff(sessionId),
    enabled: !!sessionId,
    ...options,
  });
}
