import type {
  UseMutationOptions,
  UseQueryOptions,
} from '@tanstack/react-query';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import { tasksApi } from '@/features/tasks/api/tasks';
import { queryKeys } from '@/lib/api/query-keys';
import type { AgentTriggerRequest, AgentTriggerResponse } from '@/types/agent';
import type { FileSearchResponse } from '@/types/environment';
import type { Event } from '@/types/event.ts';
import type { GitDiffResponse } from '@/types/git-diff';
import type { TaskMetadata } from '@/types/task';

// Query hooks
export function useTasksList(
  params?: {
    activeOnly?: boolean;
    limit?: number;
    offset?: number;
  },
  options?: Omit<UseQueryOptions<Array<TaskMetadata>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasksList(params),
    queryFn: () => tasksApi.listTasks(params),
    ...options,
  });
}

export function useTask(
  taskId: string,
  options?: Omit<UseQueryOptions<TaskMetadata>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskDetail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: !!taskId,
    ...options,
  });
}

// Mutation hooks
export function useCreateTask(
  options?: UseMutationOptions<
    AgentTriggerResponse,
    Error,
    AgentTriggerRequest
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: AgentTriggerRequest) => tasksApi.createTask(request),
    onSuccess: (data, variables, context) => {
      // Invalidate tasks list to refresh with new task
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks() });
      options?.onSuccess?.(data, variables, context);
    },
    ...options,
  });
}

export function useSendMessage(
  options?: UseMutationOptions<
    AgentTriggerResponse,
    Error,
    { taskId: string; message: string }
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ taskId, message }) => tasksApi.sendMessage(taskId, message),
    onSuccess: (data, variables, context) => {
      // Invalidate messages for this task
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskMessages(variables.taskId),
      });
      // Invalidate task detail to update metadata
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskDetail(variables.taskId),
      });
      // Invalidate task events to show new events
      queryClient.invalidateQueries({
        queryKey: queryKeys.taskEvents(variables.taskId),
      });
      options?.onSuccess?.(data, variables, context);
    },
    ...options,
  });
}

export function useStopAgent(
  options?: UseMutationOptions<
    { status: string; task_id: string },
    Error,
    string
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (taskId: string) => tasksApi.stopAgent(taskId),
    onSuccess: (data, taskId, context) => {
      // Invalidate all task-related queries
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks() });
      options?.onSuccess?.(data, taskId, context);
    },
    ...options,
  });
}

export function useTaskEvents(
  taskId: string,
  params?: {
    limit?: number;
    offset?: number;
  },
  options?: Omit<UseQueryOptions<Array<Event>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskEvents(taskId, params),
    queryFn: () => tasksApi.getTaskEvents(taskId, params),
    enabled: !!taskId,
    ...options,
  });
}

export function useTaskGitDiff(
  taskId: string,
  options?: Omit<UseQueryOptions<GitDiffResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskGitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: !!taskId,
    ...options,
  });
}

export function useTaskFileSearch(
  taskId: string,
  query: string,
  options?: Omit<UseQueryOptions<FileSearchResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskFileSearch(taskId, query),
    queryFn: () => tasksApi.searchTaskFiles(taskId, { query }),
    enabled: !!taskId && query.length >= 3,
    staleTime: 30 * 1000, // 30 seconds
    ...options,
  });
}
