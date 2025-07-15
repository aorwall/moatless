import { CreateTaskRequestSchema, DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'
import { useBranches, useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { RepositorySelector } from '@/features/tasks/components/chat-input/repository-selector'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { useZodForm } from '@/lib/hooks/use-zod-form'
import { getErrorMessage } from '@/lib/utils/error'

export function CreateTaskForm() {
  const navigate = useNavigate()
  const [selectedRepositoryId, setSelectedRepositoryId] = useState<string>('')
  const [selectedBranch, setSelectedBranch] = useState<string>('__none__')

  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList()

  const { data: branches, isLoading: branchesLoading } = useBranches(selectedRepositoryId)

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
    }

    // Validate using schema
    const result = validate(taskData)

    if (!result.success) {
      // Show first error as a toast
      const firstError = result.error.errors[0]
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
          <RepositorySelector
            repositories={repositories || []}
            loading={repositoriesLoading}
            selectedRepositoryId={selectedRepositoryId}
            onRepositoryChange={handleRepositoryChange}
          />
          {errors.repositoryId && <p className="text-sm text-destructive">{errors.repositoryId}</p>}
        </div>

        {selectedRepositoryId && (
          <div className="space-y-2">
            <Label htmlFor="branch" className="text-sm font-medium text-foreground">
              Select Branch (optional)
            </Label>
            <Select value={selectedBranch} onValueChange={setSelectedBranch}>
              <SelectTrigger>
                <SelectValue
                  placeholder={
                    branchesLoading
                      ? 'Loading branches...'
                      : 'Select branch or leave empty to branch from main'
                  }
                />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">
                  <span className="text-muted-foreground">None (branch from main)</span>
                </SelectItem>
                {branches?.branches.map((branch) => (
                  <SelectItem key={branch} value={branch}>
                    <div className="flex items-center gap-2">
                      <span>{branch}</span>
                      {branch === branches.currentBranch && (
                        <span className="text-xs text-status-success">(current)</span>
                      )}
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              If no branch is selected, the task will branch out from main
            </p>
          </div>
        )}

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
