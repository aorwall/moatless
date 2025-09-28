import { CreateTaskRequestSchema, DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import {
  BranchPopoverContent,
  PopoverSelector,
  SearchablePopoverContent,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useBranchSelector,
  useRepositorySelector,
} from '@/lib/components/chat/chat-input/selectors'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Label } from '@/lib/components/ui/label'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { getErrorMessage } from '@/lib/utils/error'

export function CreateTaskForm() {
  const navigate = useNavigate()
  const [selectedRepositoryId, setSelectedRepositoryId] = useState<string>('')
  const [selectedBranch, setSelectedBranch] = useState<string>('__none__')

  // Use selector hooks
  const agentModeSelector = useAgentModeSelector('edit')
  const repoSelector = useRepositorySelector(selectedRepositoryId, setSelectedRepositoryId, true)
  const branchSelector = useBranchSelector(
    selectedRepositoryId,
    selectedBranch,
    setSelectedBranch,
    true,
  )

  // Use schema validation
  const { errors, validate } = useZodForm({ schema: CreateTaskRequestSchema })

  const createTask = useCreateTask({
    onSuccess: (data) => {
      navigate({ to: `/tasks/${data.taskId}` })
    },
    onError: () => {
      toast.error('Failed to create task. Please try again.')
    },
  })

  const handleRepositoryChange = (repoId: string) => {
    setSelectedRepositoryId(repoId)
    setSelectedBranch('__none__') // Reset branch selection when repository changes
  }

  const handleCreateTask = () => {
    const taskData = {
      agentType: 'claude-code',
      message: '',
      repositoryId: selectedRepositoryId,
      branchName: selectedBranch === '__none__' ? undefined : selectedBranch,
      model: DEFAULT_CLAUDE_MODEL, // Use default model for now
      agentMode: agentModeSelector.value,
    }

    // Validate using schema
    const result = validate(taskData)

    if (!result.success) {
      // Show first error as a toast
      const firstError = result.error.issues[0]
      if (firstError) {
        toast.error(firstError.message)
      }
      return
    }

    createTask.mutate(result.data)
  }

  return (
    <Card className="mx-auto max-w-2xl">
      <CardHeader>
        <CardTitle className="text-center text-2xl">Create New Task</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="space-y-2">
          <Label htmlFor="repository" className="text-sm font-medium text-foreground">
            Select Repository
          </Label>
          <PopoverSelector
            icon={repoSelector.icon}
            value={repoSelector.value}
            displayValue={repoSelector.displayValue}
            tooltip={repoSelector.tooltip}
            loading={repoSelector.loading}
            disabled={repoSelector.disabled}
            className="w-full justify-start"
          >
            <SearchablePopoverContent
              items={repoSelector.items}
              value={repoSelector.value}
              onChange={handleRepositoryChange}
              placeholder="Search repositories..."
              emptyText="No repositories found"
            />
          </PopoverSelector>
          {errors.repositoryId && <p className="text-sm text-destructive">{errors.repositoryId}</p>}
        </div>

        {selectedRepositoryId && (
          <div className="space-y-2">
            <Label htmlFor="branch" className="text-sm font-medium text-foreground">
              Select Branch (optional)
            </Label>
            <PopoverSelector
              icon={branchSelector.icon}
              value={branchSelector.value}
              displayValue={branchSelector.displayValue}
              tooltip={branchSelector.tooltip}
              loading={branchSelector.loading}
              disabled={branchSelector.disabled}
              className="w-full justify-start"
            >
              <BranchPopoverContent
                items={[
                  { id: '__none__', label: 'None (branch from main)' },
                  ...branchSelector.items,
                ]}
                value={branchSelector.value}
                onChange={setSelectedBranch}
                currentBranch={branchSelector.currentBranch}
                enableSearch={true}
              />
            </PopoverSelector>
            <p className="text-xs text-muted-foreground">
              If no branch is selected, the task will branch out from main
            </p>
          </div>
        )}

        <div className="space-y-2">
          <Label htmlFor="agentMode" className="text-sm font-medium text-foreground">
            Agent Mode
          </Label>
          <PopoverSelector
            icon={agentModeSelector.icon}
            value={agentModeSelector.value}
            displayValue={agentModeSelector.displayValue}
            tooltip={agentModeSelector.tooltip}
            loading={agentModeSelector.loading}
            disabled={agentModeSelector.disabled}
            className="w-full justify-start"
          >
            <SimpleListPopoverContent
              items={agentModeSelector.items}
              value={agentModeSelector.value}
              onChange={agentModeSelector.onChange}
            />
          </PopoverSelector>
          <p className="text-xs text-muted-foreground">
            Choose between Edit mode (makes changes) or Plan mode (creates plans)
          </p>
        </div>

        {createTask.error && (
          <div className="rounded-md bg-status-error/10 p-4 text-sm text-status-error">
            {getErrorMessage(createTask.error)}
          </div>
        )}

        <Button
          onClick={handleCreateTask}
          disabled={!selectedRepositoryId || createTask.isPending}
          className="w-full"
          size="lg"
        >
          {createTask.isPending ? 'Creating...' : 'Create Task'}
        </Button>
      </CardContent>
    </Card>
  )
}
