import type {
  CreateTaskRequest,
  MessageContextInput,
  WorkspaceRepoInput,
} from '@moatless/api-client'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'
import type { AgentType } from '@/features/tasks/components/chat/chat-input/selectors'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { useCreateWorkspace } from '@/features/workspaces'
import { queryKeys } from '@/lib/api/query-keys'
import { getErrorMessage } from '@/lib/utils/error'
import type { AdditionalRepoDraft } from './additional-repos-field'

/** Everything the composer needs to submit a task, gathered in one struct. */
export interface CodeTaskSubmitInput {
  agentType: AgentType
  repositoryId: string
  /** Extra repos to check out alongside the primary; empty = single-repo path. */
  additionalRepos: AdditionalRepoDraft[]
  /** Repositories list, used only to name the ad-hoc multi-repo workspace. */
  repositories: { id: string; name: string }[] | undefined
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
 * Single-repo (the common path) is unchanged: the create-task request carries
 * only `repositoryId`, so the backend derives the repository's shadow
 * workspace. When the user attaches extra repos, we first materialize an ad-hoc
 * multi-repo workspace (primary + extras) and then pin the task to it via
 * `workspaceId`.
 */
export function useCodeTaskSubmit() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const createTask = useCreateTask({
    onSuccess: async (data) => {
      // Invalidate new task data (including messages) - lists already handled by hook
      await queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(data.taskId) })
      await navigate({ to: '/tasks/$task-id', params: { 'task-id': data.taskId } })
    },
    onError: (error) => {
      toast.error('Failed to create task', { description: getErrorMessage(error) })
    },
  })
  const createWorkspace = useCreateWorkspace()

  const submit = (input: CodeTaskSubmitInput) => {
    const onDone = { onSuccess: input.onSubmitted }

    // Guard against the primary repo being re-selected into the extra list.
    const extraRepos = input.additionalRepos.filter((r) => r.repositoryId !== input.repositoryId)

    if (extraRepos.length === 0) {
      createTask.mutate(buildTaskInput(input, undefined), onDone)
      return
    }

    createWorkspace.mutate(
      {
        name: buildWorkspaceName(input.repositoryId, extraRepos, input.repositories),
        repos: buildWorkspaceRepos(input.repositoryId, input.branch, extraRepos),
      },
      {
        onSuccess: (workspace) => createTask.mutate(buildTaskInput(input, workspace.id), onDone),
        onError: (error) => {
          toast.error('Failed to create workspace', { description: getErrorMessage(error) })
        },
      },
    )
  }

  return { submit, isSubmitting: createTask.isPending || createWorkspace.isPending }
}

// Build the create-task payload, attaching workspaceId only on the multi-repo
// path. Optional fields are omitted (not sent as undefined) to match the
// original single-repo request shape exactly.
function buildTaskInput(
  input: CodeTaskSubmitInput,
  workspaceId: string | undefined,
): CreateTaskRequest {
  return {
    agentType: input.agentType,
    repositoryId: input.repositoryId,
    fileIds: input.fileIds,
    agentMode: input.agentMode,
    ...(workspaceId !== undefined && { workspaceId }),
    ...(input.message !== undefined && { message: input.message }),
    ...(input.model !== undefined && { model: input.model }),
    ...(input.contexts.length > 0 && { contexts: input.contexts }),
    ...(input.branch !== undefined && { branch: input.branch }),
    ...(input.commandName !== undefined && { commandName: input.commandName }),
    ...(input.templateId !== undefined && { taskTemplateId: input.templateId }),
  }
}

// Build the workspace placement list: primary repo first (branch from the
// task's branch selector), then extras with their per-placement branch/mount
// overrides. Empty overrides are omitted so the backend applies its defaults.
function buildWorkspaceRepos(
  repositoryId: string,
  branch: string | undefined,
  extraRepos: AdditionalRepoDraft[],
): WorkspaceRepoInput[] {
  return [
    { repositoryId, isPrimary: true, ...(branch !== undefined && { branch }) },
    ...extraRepos.map((r, i) => ({
      repositoryId: r.repositoryId,
      position: i + 1,
      ...(r.branch !== '' && { branch: r.branch }),
      ...(r.mountName !== '' && { mountName: r.mountName }),
    })),
  ]
}

// Derive a readable name for the ad-hoc multi-repo workspace from the repo
// names (e.g. "api + web"). Falls back to ids when names aren't loaded yet.
function buildWorkspaceName(
  primaryRepositoryId: string,
  extraRepos: AdditionalRepoDraft[],
  repositories: { id: string; name: string }[] | undefined,
): string {
  const byId = new Map(repositories?.map((r) => [r.id, r.name]))
  const nameOf = (id: string) => byId.get(id) ?? id
  return [primaryRepositoryId, ...extraRepos.map((r) => r.repositoryId)].map(nameOf).join(' + ')
}
