import type { CreateTaskRequest, MessageContextInput } from '@moatless/api-client'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'
import type { AgentType } from '@/features/tasks/components/chat/chat-input/selectors'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { getErrorMessage } from '@/lib/utils/error'

/** Everything the composer needs to submit a task, gathered in one struct. */
export interface CodeTaskSubmitInput {
  agentType: AgentType
  workspaceId: string
  fileIds: string[]
  agentMode: 'edit' | 'plan'
  message: string | undefined
  model: string | undefined
  contexts: MessageContextInput[]
  branch: string | undefined
  commandName: string | undefined
  templateId: string | undefined
  /** Called after the task-create mutation succeeds (clear the composer). */
  onSubmitted: () => void
}

/**
 * Task-creation orchestration for the code composer.
 *
 * Submits a task against the selected workspace. The workspace's repositories
 * (its primary repo in particular) drive the task.
 */
export function useCodeTaskSubmit() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const createTask = useCreateTask({
    onSuccess: async (data) => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(data.taskId) })
      await navigate({ to: '/tasks/$task-id', params: { 'task-id': data.taskId } })
    },
    onError: (error) => {
      toast.error('Failed to create task', { description: getErrorMessage(error) })
    },
  })

  const submit = (input: CodeTaskSubmitInput) => {
    createTask.mutate(buildTaskInput(input), { onSuccess: input.onSubmitted })
  }

  return { submit, isSubmitting: createTask.isPending }
}

function buildTaskInput(input: CodeTaskSubmitInput): CreateTaskRequest {
  return {
    agentType: input.agentType,
    workspaceId: input.workspaceId,
    fileIds: input.fileIds,
    agentMode: input.agentMode,
    ...(input.message !== undefined && { message: input.message }),
    ...(input.model !== undefined && { model: input.model }),
    ...(input.contexts.length > 0 && { contexts: input.contexts }),
    ...(input.branch !== undefined && { branch: input.branch }),
    ...(input.commandName !== undefined && { commandName: input.commandName }),
    ...(input.templateId !== undefined && { taskTemplateId: input.templateId }),
  }
}
