import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'

import { useBranches, useEnvironmentsList } from '@/features/environments/hooks/use-environments'
import { EnvironmentSelector } from '@/features/tasks/components/chat-input/environment-selector'
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
import { getErrorMessage } from '@/lib/utils/error'

export function CreateTaskForm() {
  const navigate = useNavigate()
  const [selectedEnvironmentId, setSelectedEnvironmentId] = useState<string>('')
  const [selectedBranch, setSelectedBranch] = useState<string>('__none__')

  const { data: environments, isLoading: environmentsLoading } = useEnvironmentsList()

  const { data: branches, isLoading: branchesLoading } = useBranches(selectedEnvironmentId)

  const createTask = useCreateTask({
    onSuccess: (data) => {
      navigate({ to: `/tasks/${data.taskId}` })
    },
  })

  const handleEnvironmentChange = (envId: string) => {
    setSelectedEnvironmentId(envId)
    setSelectedBranch('__none__') // Reset branch selection when environment changes
  }

  const handleCreateTask = () => {
    if (!selectedEnvironmentId) {
      return
    }

    createTask.mutate({
      agentType: 'claude_code',
      message: '',
      environmentId: selectedEnvironmentId,
      branchName: selectedBranch === '__none__' ? undefined : selectedBranch,
    })
  }

  return (
    <Card className="mx-auto max-w-2xl">
      <CardHeader>
        <CardTitle className="text-center text-2xl">Create New Task</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="space-y-2">
          <Label htmlFor="environment" className="text-sm font-medium text-gray-700">
            Select Environment
          </Label>
          <EnvironmentSelector
            environments={environments || []}
            loading={environmentsLoading}
            selectedEnvironmentId={selectedEnvironmentId}
            onEnvironmentChange={handleEnvironmentChange}
          />
        </div>

        {selectedEnvironmentId && (
          <div className="space-y-2">
            <Label htmlFor="branch" className="text-sm font-medium text-gray-700">
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
                  <span className="text-gray-600">None (branch from main)</span>
                </SelectItem>
                {branches?.branches.map((branch) => (
                  <SelectItem key={branch.name} value={branch.name}>
                    <div className="flex items-center gap-2">
                      <span>{branch.name}</span>
                      {branch.is_current && (
                        <span className="text-xs text-green-600">(current)</span>
                      )}
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-gray-500">
              If no branch is selected, the task will branch out from main
            </p>
          </div>
        )}

        {createTask.error && (
          <div className="rounded-md bg-red-50 p-4 text-sm text-red-800">
            {getErrorMessage(createTask.error)}
          </div>
        )}

        <Button
          onClick={handleCreateTask}
          disabled={!selectedEnvironmentId || createTask.isPending}
          className="w-full"
          size="lg"
        >
          {createTask.isPending ? 'Creating...' : 'Create Task'}
        </Button>
      </CardContent>
    </Card>
  )
}
