import type {
  CommandResponse,
  CreateTaskRequest,
  CreateTaskResponse,
  ListTasksResponse,
  LiveStatus,
  MessageContextInput,
  SendMessageResponse,
  TaskResponse,
  ToolCallDetailResponse,
  UiMessage,
  UpdateTaskTtlRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { liveStatusHandler } from '@moatless/api-client/sandbox/sandbox/sandbox'
import type {
  InfiniteData,
  QueryClient,
  UseMutationOptions,
  UseQueryOptions,
} from '@tanstack/react-query'
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import type { MessagesPage } from '@/features/tasks/api/tasks'
import { tasksApi } from '@/features/tasks/api/tasks'
import { updateTaskInLists } from '@/features/tasks/utils/query-helpers'
import { queryKeys } from '@/lib/api/query-keys'
import { swapMessageUuid } from '@/lib/api/task-cache'
import type { AskUserQuestionResponse, ExitPlanModeResponse } from '@/lib/types/user-input'

export const MESSAGES_PAGE_SIZE = 200

type MessagesInfiniteData = InfiniteData<MessagesPage, number | undefined>

/**
 * Update every page of an InfiniteData<MessagesPage> with `updater`.
 * Used by optimistic mutations that need to add/remove rows.
 */
function mutateMessagePages(
  data: MessagesInfiniteData | undefined,
  updater: (messages: UiMessage[], pageIndex: number, pageCount: number) => UiMessage[],
): MessagesInfiniteData | undefined {
  if (!data) {
    return data
  }
  return {
    ...data,
    pages: data.pages.map((page, idx, pages) => ({
      ...page,
      messages: updater(page.messages, idx, pages.length),
    })),
  }
}

/**
 * Task Query Invalidation Strategy:
 *
 * - queryKeys.tasks() - Invalidates all task queries (list + all individual tasks)
 * - queryKeys.task(taskId) - Invalidates all data for a specific task (detail, messages, events, etc.)
 *
 * For mutations:
 * - Always invalidate queryKeys.task(taskId) to refresh all task data
 * - Also invalidate queryKeys.tasks() to update the task list
 */

// Query hooks
export function useTasksList(
  params?: {
    activeOnly?: boolean
    status?: 'open' | 'error' | 'closed'
    limit?: number
    offset?: number
    tag?: string
    createdBy?: string
    followedBy?: string
    repositoryId?: string
    parentTaskId?: string
  },
  options?: Omit<UseQueryOptions<ListTasksResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.list(params),
    queryFn: () => tasksApi.listTasks(params),
    ...options,
  })
}

export function useTask(
  taskId: string,
  options?: Omit<UseQueryOptions<TaskResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    enabled: !!taskId,
    ...options,
  })
}

/**
 * Mark a task's conversation as read while it is being viewed. Fires once when
 * the task is opened (clearing any unread indicator immediately) and again when
 * navigating away, so `lastReadAt` captures anything that streamed in while the
 * user was looking at it. Optimistically patches the task caches so the sidebar
 * updates without waiting for a refetch. Fire-and-forget — read receipts are
 * low-stakes, so a failed request just gets retried on the next view.
 */
export function useMarkTaskReadOnView(taskId: string): void {
  const queryClient = useQueryClient()

  useEffect(() => {
    if (!taskId) {
      return
    }
    const mark = () => {
      const readAt = new Date().toISOString()
      updateTaskInLists(queryClient, taskId, { lastReadAt: readAt })
      queryClient.setQueryData<TaskResponse>(queryKeys.tasks.detail(taskId), (prev) =>
        prev ? { ...prev, lastReadAt: readAt } : prev,
      )
      tasksApi.markTaskRead(taskId).catch(() => {
        // Best-effort — a failed read receipt is retried on the next view.
      })
    }
    mark()
    return mark
  }, [taskId, queryClient])
}

/**
 * Follow or unfollow a task. Following adds it to the user's personal task
 * list (the sidebar, driven by a `followedBy` filter); unfollowing removes it.
 * Optimistically patches the `isFollowing` flag on the task detail cache, then
 * invalidates the task lists so followed/unfollowed rows appear or disappear.
 */
export function useToggleTaskFollow(
  taskId: string,
  options?: UseMutationOptions<
    { following: boolean },
    Error,
    boolean,
    { previousTask: TaskResponse | undefined }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<
    { following: boolean },
    Error,
    boolean,
    { previousTask: TaskResponse | undefined }
  >({
    ...options,
    mutationKey: ['tasks', taskId, 'toggleFollow'],
    mutationFn: (following: boolean) =>
      following ? tasksApi.followTask(taskId) : tasksApi.unfollowTask(taskId),
    onMutate: async (following) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.detail(taskId) })

      const previousTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))

      queryClient.setQueryData<TaskResponse>(queryKeys.tasks.detail(taskId), (prev) =>
        prev ? { ...prev, isFollowing: following } : prev,
      )
      updateTaskInLists(queryClient, taskId, { isFollowing: following })

      return { previousTask }
    },
    onError: (error, following, context, mutationContext) => {
      if (context?.previousTask) {
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), context.previousTask)
        updateTaskInLists(queryClient, taskId, context.previousTask)
      }
      options?.onError?.(error, following, context, mutationContext)
    },
    onSettled: async () => {
      // Refetch task lists so the (un)followed task appears in or drops out of
      // the sidebar's followedBy-filtered list.
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.lists(),
        exact: false,
        type: 'active',
      })
    },
    onSuccess: (data, following, onMutateResult, mutationContext) => {
      options?.onSuccess?.(data, following, onMutateResult, mutationContext)
    },
  })
}

// Mutation hooks
export function useCreateTask(
  options?: UseMutationOptions<CreateTaskResponse, Error, CreateTaskRequest>,
): ReturnType<typeof useMutation<CreateTaskResponse, Error, CreateTaskRequest>> {
  const queryClient = useQueryClient()

  return useMutation<CreateTaskResponse, Error, CreateTaskRequest>({
    ...options,
    mutationFn: (request: CreateTaskRequest) => tasksApi.createTask(request),
    onSuccess: async (data, variables, onMutateResult, mutationContext) => {
      // Add the real task data to cache and lists
      const response = data
      if (response.task) {
        // Set task detail cache
        queryClient.setQueryData(queryKeys.tasks.detail(response.taskId), response.task)

        // Add to task lists with real data
        updateTaskInLists(queryClient, response.taskId, response.task, true)

        // Add initial message to messages cache if message was provided
        if (variables.message && response.taskId) {
          const initialMessage: UiMessage = {
            uuid: `initial-${response.taskId}`,
            role: 'user',
            content: [{ type: 'text', text: variables.message }],
            timestamp: new Date().toISOString(),
            messageType: 'user',
          }

          queryClient.setQueryData<MessagesInfiniteData>(
            queryKeys.tasks.messages(response.taskId),
            {
              pages: [{ messages: [initialMessage], hasMore: false, nextBeforeId: undefined }],
              pageParams: [undefined],
            },
          )
        }

        // Seed live status cache with initializing state for instant feedback.
        queryClient.setQueryData<LiveStatus>(queryKeys.tasks.liveStatus(response.taskId), {
          sandboxStatus: 'initializing',
          agentStatus: null,
          sandboxError: null,
        })

        // Prefetch actual live status in background (staleTime: 0 forces fetch).
        queryClient.prefetchQuery({
          queryKey: queryKeys.tasks.liveStatus(response.taskId),
          queryFn: () => unwrapResponse<LiveStatus>(liveStatusHandler(response.taskId)),
          staleTime: 0,
        })
      } else {
        // Fallback: if no task data returned, invalidate to refetch
        await queryClient.invalidateQueries({
          queryKey: queryKeys.tasks.lists(),
        })
      }

      // A new task was created — bump template usage counts so featured chips refresh
      queryClient.invalidateQueries({ queryKey: queryKeys.taskTemplates.featured() })
      queryClient.invalidateQueries({ queryKey: queryKeys.taskTemplates.list() })

      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    onError: (error, variables, onMutateResult, mutationContext) => {
      options?.onError?.(error, variables, onMutateResult, mutationContext)
    },
  })
}

interface SendMessageVariables {
  message: string
  resume?: boolean | undefined
  model?: string | undefined
  fileIds?: string[] | undefined
  /**
   * Locally-known upload metadata for the files referenced by `fileIds`.
   * Used only for the optimistic message render so file chips appear
   * immediately; not sent to the backend (the backend re-resolves filenames
   * from `file_metadata`).
   */
  fileUploads?:
    | Array<{ fileId: string; filename: string; mimeType: string; size: number }>
    | undefined
  commandName?: string | undefined
  agentMode?: string | undefined
  contexts?: MessageContextInput[] | undefined
  toolUseId?: string | undefined
  toolName?: 'AskUserQuestion' | 'ExitPlanMode' | undefined
  toolResponse?: AskUserQuestionResponse | ExitPlanModeResponse | undefined
  /**
   * When true, the next agent run starts a fresh session (no `--resume`).
   * Per-message; the chat composer clears the toggle after each successful send.
   */
  startFresh?: boolean | undefined
}

export function useSendMessage(
  taskId: string,
  options?: UseMutationOptions<
    SendMessageResponse,
    Error,
    SendMessageVariables,
    {
      previousMessages: MessagesInfiniteData | undefined
      previousTask: TaskResponse | undefined
      optimisticUuid: string
    }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<
    SendMessageResponse,
    Error,
    SendMessageVariables,
    {
      previousMessages: MessagesInfiniteData | undefined
      previousTask: TaskResponse | undefined
      optimisticUuid: string
    }
  >({
    ...options,
    mutationKey: ['tasks', taskId, 'sendMessage'],
    mutationFn: (variables) => tasksApi.sendMessage(taskId, variables),
    onMutate: async (variables) => {
      // Cancel any outgoing refetches (so they don't overwrite our optimistic update)
      await queryClient.cancelQueries({
        queryKey: queryKeys.tasks.messages(taskId),
      })
      await queryClient.cancelQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })

      // Snapshot current cache for rollback
      const previousMessages = queryClient.getQueryData<MessagesInfiniteData>(
        queryKeys.tasks.messages(taskId),
      )
      const previousTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))

      // Create optimistic message — mark as queued when the agent is already running,
      // since the backend will save it with agentSessionId = NULL
      const liveStatus = queryClient.getQueryData<LiveStatus>(queryKeys.tasks.liveStatus(taskId))
      const isQueued = liveStatus?.agentStatus === 'running'

      // Build optimistic contexts: caller-supplied (preview/element) +
      // synthesised File contexts from locally-known uploads. The server
      // reply via SSE / refresh will replace these with authoritative rows.
      const optimisticContexts: MessageContextInput[] = [
        ...(variables.contexts ?? []),
        ...(variables.fileUploads ?? []).map<MessageContextInput>((upload) => ({
          type: 'file',
          referenceId: upload.fileId,
          data: {
            filename: upload.filename,
            mimeType: upload.mimeType,
            size: upload.size,
          },
        })),
      ]

      const optimisticUuid = `temp-message-${Date.now()}`
      const optimisticMessage: UiMessage = {
        uuid: optimisticUuid,
        role: 'user',
        content: [{ type: 'text', text: variables.message }],
        timestamp: new Date().toISOString(),
        messageType: 'user',
        ...(isQueued ? { isQueued: true } : {}),
        ...(variables.commandName ? { commandName: variables.commandName } : {}),
        ...(optimisticContexts.length > 0 ? { contexts: optimisticContexts } : {}),
      }

      // Append the optimistic row to the LAST page (newest). If no pages are
      // cached yet, seed an initial page.
      queryClient.setQueryData<MessagesInfiniteData>(queryKeys.tasks.messages(taskId), (data) => {
        if (!data) {
          return {
            pages: [{ messages: [optimisticMessage], hasMore: false, nextBeforeId: undefined }],
            pageParams: [undefined],
          }
        }
        return mutateMessagePages(data, (messages, idx, count) =>
          idx === count - 1 ? [...messages, optimisticMessage] : messages,
        )
      })

      return { previousMessages, previousTask, optimisticUuid }
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Swap the provisional uuid for the server-assigned one so the later
      // `message.upsert` for this Message reconciles idempotently by uuid
      // instead of appearing as a duplicate. SSE keeps the rest of the cache
      // fresh — no invalidation here.
      if (data.uuid && onMutateResult?.optimisticUuid) {
        swapMessageUuid(queryClient, taskId, onMutateResult.optimisticUuid, data.uuid)
      }
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    onError: (error, variables, onMutateResult, mutationContext) => {
      // Rollback optimistic updates on error
      if (onMutateResult?.previousMessages !== undefined) {
        queryClient.setQueryData(queryKeys.tasks.messages(taskId), onMutateResult.previousMessages)
      }
      if (onMutateResult?.previousTask) {
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), onMutateResult.previousTask)
      }
      options?.onError?.(error, variables, onMutateResult, mutationContext)
    },
  })
}

export function useStopAgent(
  taskId: string,
  options?: UseMutationOptions<{ status: string; taskId: string }, Error>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    ...options,
    mutationKey: ['tasks', taskId, 'stop'],
    mutationFn: () => tasksApi.stopAgent(taskId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.lists(),
      })
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useDeleteMessage(
  taskId: string,
  options?: UseMutationOptions<
    { status: 'deleted' },
    Error,
    string,
    { previousMessages: MessagesInfiniteData | undefined }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<
    { status: 'deleted' },
    Error,
    string,
    { previousMessages: MessagesInfiniteData | undefined }
  >({
    ...options,
    mutationKey: ['tasks', taskId, 'deleteMessage'],
    mutationFn: (messageId: string) => tasksApi.deleteMessage(taskId, messageId),
    onMutate: async (messageId) => {
      await queryClient.cancelQueries({
        queryKey: queryKeys.tasks.messages(taskId),
      })

      const previousMessages = queryClient.getQueryData<MessagesInfiniteData>(
        queryKeys.tasks.messages(taskId),
      )

      // Optimistically remove the message from whichever page contains it.
      queryClient.setQueryData<MessagesInfiniteData>(queryKeys.tasks.messages(taskId), (data) =>
        mutateMessagePages(data, (messages) => messages.filter((m) => m.uuid !== messageId)),
      )

      return { previousMessages }
    },
    onError: (error, _messageId, context, mutation) => {
      // Rollback on error
      if (context?.previousMessages !== undefined) {
        queryClient.setQueryData(queryKeys.tasks.messages(taskId), context.previousMessages)
      }
      options?.onError?.(error, _messageId, context, mutation)
    },
  })
}

export function useTaskMessages(taskId: string, options?: { enabled?: boolean }) {
  return useInfiniteQuery({
    queryKey: queryKeys.tasks.messages(taskId),
    queryFn: ({ pageParam }) =>
      tasksApi.getTaskMessages(taskId, {
        ...(pageParam !== undefined && { beforeId: pageParam }),
        limit: MESSAGES_PAGE_SIZE,
      }),
    initialPageParam: undefined as number | undefined,
    // Older pages are paginated. Newer messages arrive via targeted refetch
    // of the newest page (see `refreshNewestMessagesPage`).
    getPreviousPageParam: (firstPage) => (firstPage.hasMore ? firstPage.nextBeforeId : undefined),
    getNextPageParam: () => undefined,
    // Cap the cache so very long tasks don't drag the DOM; oldest pages drop
    // when the user scrolls past the limit and re-fetch on demand.
    maxPages: 10,
    enabled: !!taskId && options?.enabled !== false,
  })
}

/**
 * Refetch only the newest page of messages and merge it into the cached
 * infinite query, leaving older pages untouched. Used by SSE event handlers
 * so that incoming agent activity does not reset the user's scroll position
 * by invalidating the entire infinite query.
 */
export async function refreshNewestMessagesPage(
  queryClient: QueryClient,
  taskId: string,
): Promise<void> {
  const newest = await tasksApi.getTaskMessages(taskId, { limit: MESSAGES_PAGE_SIZE })
  queryClient.setQueryData<MessagesInfiniteData>(queryKeys.tasks.messages(taskId), (data) => {
    if (!data || data.pages.length === 0) {
      return {
        pages: [newest],
        pageParams: [undefined],
      }
    }
    const pages = [...data.pages]
    const pageParams = [...data.pageParams]
    pages[pages.length - 1] = newest
    pageParams[pageParams.length - 1] = undefined
    return { ...data, pages, pageParams }
  })
}

/**
 * Fetch the full input + result for a single tool call. Disabled until the
 * caller actually expands the tool call so list pages stay slim. The query is
 * keyed on `tool_use_id`, so re-collapsing and re-expanding within the same
 * route hits the cache, and SSE-driven list refreshes don't drop this data.
 */
export function useTaskToolCall(
  taskId: string,
  toolUseId: string | null | undefined,
  enabled: boolean,
  options?: Omit<UseQueryOptions<ToolCallDetailResponse>, 'queryKey' | 'queryFn' | 'enabled'>,
) {
  const id = toolUseId ?? ''
  return useQuery({
    queryKey: queryKeys.tasks.toolCall(taskId, id),
    queryFn: () => tasksApi.getToolCallDetail(taskId, id),
    enabled: enabled && !!taskId && !!id,
    staleTime: Number.POSITIVE_INFINITY,
    ...options,
  })
}

export function useTaskCommands(
  taskId: string,
  options?: Omit<UseQueryOptions<CommandResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tasks.commands(taskId),
    queryFn: () => tasksApi.getTaskCommands(taskId),
    enabled: !!taskId,
    ...options,
  })
}

export function useCloseTask(taskId: string, options?: UseMutationOptions<TaskResponse, Error>) {
  const queryClient = useQueryClient()

  return useMutation<TaskResponse, Error, void>({
    ...options,
    mutationKey: ['tasks', taskId, 'close'],
    mutationFn: () => tasksApi.closeTask(taskId),
    onMutate: () => {
      // Snapshot the previous task data
      const previousTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))

      // Optimistically remove the task from all cached lists
      if (previousTask) {
        // Update task status to 'closed' in detail cache
        const updatedTask = { ...previousTask, status: 'closed' as const }
        queryClient.setQueryData<TaskResponse>(queryKeys.tasks.detail(taskId), updatedTask)

        // Remove the task from all cached lists for immediate UI feedback
        updateTaskInLists(queryClient, taskId, null)
      }

      // Return context with snapshot for potential rollback
      return { previousTask }
    },
    onError: (err, variables, onMutateResult, mutationContext) => {
      // Rollback on error
      if (
        onMutateResult &&
        typeof onMutateResult === 'object' &&
        'previousTask' in onMutateResult &&
        onMutateResult.previousTask
      ) {
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), onMutateResult.previousTask)
        updateTaskInLists(queryClient, taskId, onMutateResult.previousTask)
      }
      options?.onError?.(err, variables, onMutateResult, mutationContext)
    },
    onSettled: async () => {
      // Only force refetch task lists - events will handle task-specific updates
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.lists(),
        exact: false,
        type: 'active',
      })
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

/**
 * Reopen a previously closed task (flips `closed` → `open`).
 *
 * The backend does not provision a sandbox here — that happens on the next
 * start/message, at which point any retained S3 state is restored. This mirrors
 * `useCloseTask` optimistically: the detail cache flips back to `open` and the
 * task lists are refetched so the reopened row reappears.
 */
export function useReopenTask(taskId: string, options?: UseMutationOptions<TaskResponse, Error>) {
  const queryClient = useQueryClient()

  return useMutation<TaskResponse, Error, void>({
    ...options,
    mutationKey: ['tasks', taskId, 'reopen'],
    mutationFn: () => tasksApi.reopenTask(taskId),
    onMutate: () => {
      // Snapshot the previous task data for potential rollback.
      const previousTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))

      // Optimistically flip the detail cache to 'open'.
      if (previousTask) {
        const updatedTask = { ...previousTask, status: 'open' as const }
        queryClient.setQueryData<TaskResponse>(queryKeys.tasks.detail(taskId), updatedTask)
        updateTaskInLists(queryClient, taskId, updatedTask)
      }

      return { previousTask }
    },
    onError: (err, variables, onMutateResult, mutationContext) => {
      // Rollback on error.
      if (
        onMutateResult &&
        typeof onMutateResult === 'object' &&
        'previousTask' in onMutateResult &&
        onMutateResult.previousTask
      ) {
        queryClient.setQueryData(queryKeys.tasks.detail(taskId), onMutateResult.previousTask)
        updateTaskInLists(queryClient, taskId, onMutateResult.previousTask)
      }
      options?.onError?.(err, variables, onMutateResult, mutationContext)
    },
    onSettled: async () => {
      // Refetch task lists so the reopened row reappears where relevant.
      await queryClient.refetchQueries({
        queryKey: queryKeys.tasks.lists(),
        exact: false,
        type: 'active',
      })
    },
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useUpdateTaskVisibility(
  taskId: string,
  options?: UseMutationOptions<{ success: true }, Error, 'private' | 'public'>,
) {
  const queryClient = useQueryClient()

  return useMutation<{ success: true }, Error, 'private' | 'public'>({
    ...options,
    mutationKey: ['tasks', taskId, 'updateVisibility'],
    mutationFn: (visibility) => tasksApi.updateTaskVisibility(taskId, visibility),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useUpdateTaskMetadata(
  taskId: string,
  options?: UseMutationOptions<
    { success: boolean },
    Error,
    { name?: string; description?: string }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation<{ success: boolean }, Error, { name?: string; description?: string }>({
    ...options,
    mutationKey: ['tasks', taskId, 'updateMetadata'],
    mutationFn: (updates) => tasksApi.updateTaskMetadata(taskId, updates),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useUpdateTaskTtl(
  taskId: string,
  options?: UseMutationOptions<{ success: true }, Error, UpdateTaskTtlRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation<{ success: true }, Error, UpdateTaskTtlRequest>({
    ...options,
    mutationKey: ['tasks', taskId, 'updateTtl'],
    mutationFn: (request) => tasksApi.updateTaskTtl(taskId, request),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(taskId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}

export function useDeleteTask(options?: UseMutationOptions<{ status: string }, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation<{ status: string }, Error, string, { previousTask: TaskResponse | undefined }>(
    {
      ...options,
      mutationFn: (taskId: string) => tasksApi.deleteTask(taskId),
      onMutate: async (taskId) => {
        await queryClient.cancelQueries({ queryKey: queryKeys.tasks.lists() })

        const previousTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))

        // Optimistically remove from all cached lists
        updateTaskInLists(queryClient, taskId, null)

        return { previousTask }
      },
      onError: (error, taskId, context, mutationContext) => {
        // Rollback: restore task in lists
        if (context?.previousTask) {
          updateTaskInLists(queryClient, taskId, context.previousTask)
        }
        options?.onError?.(error, taskId, context, mutationContext)
      },
      onSettled: async () => {
        await queryClient.refetchQueries({
          queryKey: queryKeys.tasks.lists(),
          exact: false,
          type: 'active',
        })
      },
      onSuccess: (data, taskId, onMutateResult, mutationContext) => {
        queryClient.removeQueries({ queryKey: queryKeys.tasks.detail(taskId) })
        options?.onSuccess?.(data, taskId, onMutateResult, mutationContext)
      },
    },
  )
}

type DeployResult = { success: boolean; siteUrl?: string; error?: string }

export function useDeployTask(taskId: string, options?: UseMutationOptions<DeployResult, Error>) {
  const queryClient = useQueryClient()

  return useMutation<DeployResult, Error, void>({
    ...options,
    mutationKey: ['tasks', taskId, 'deploy'],
    mutationFn: () => tasksApi.deployTask(taskId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate task data to refresh any status changes
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.detail(taskId),
      })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
  })
}
