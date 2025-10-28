import { CreateTaskRequestSchema, DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import { useRepository } from '@/features/repositories'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { ContextItems, FileAttachments } from '@/lib/components/chat/chat-input'
import {
  AttachmentPopoverContent,
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

  // Task inputs hook (file uploads, context items, Figma)
  const taskInputs = useTaskInputs({
    enableContextItems: true,
    enableFigma: true,
  })

  // Fetch full repository details to check worktree/clone settings
  const { data: repositoryDetails } = useRepository(selectedRepositoryId, {
    enabled: !!selectedRepositoryId,
  })

  // Use selector hooks
  const agentModeSelector = useAgentModeSelector('edit')
  const repoSelector = useRepositorySelector(selectedRepositoryId, setSelectedRepositoryId, true)
  const branchSelector = useBranchSelector(
    selectedRepositoryId,
    selectedBranch,
    setSelectedBranch,
    true,
    repositoryDetails,
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
      model: DEFAULT_CLAUDE_MODEL,
      agentMode: agentModeSelector.value,
      fileIds: taskInputs.getFileIds(),
      contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
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

        {selectedRepositoryId && !branchSelector.hidden && (
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

        {/* File Attachments */}
        {taskInputs.uploadedFiles.length > 0 && (
          <div className="space-y-2">
            <Label className="text-sm font-medium text-foreground">Attachments</Label>
            <FileAttachments
              files={taskInputs.uploadedFiles}
              onRemove={taskInputs.handleRemoveFile}
            />
          </div>
        )}

        {/* Context Items */}
        {taskInputs.contextItems.length > 0 && (
          <div className="space-y-2">
            <Label className="text-sm font-medium text-foreground">Context Items</Label>
            <ContextItems
              contexts={taskInputs.contextItems}
              onRemove={taskInputs.handleRemoveContext}
            />
          </div>
        )}

        {/* Attachment Button */}
        <div className="flex justify-start">
          <PopoverSelector
            icon={taskInputs.attachmentSelector.icon}
            value={taskInputs.attachmentSelector.value}
            tooltip={taskInputs.attachmentSelector.tooltip}
            loading={taskInputs.attachmentSelector.loading}
            disabled={taskInputs.attachmentSelector.disabled}
          >
            <AttachmentPopoverContent
              onFileSelect={(file) => taskInputs.uploadFile.mutate(file)}
              onFigmaImport={() => taskInputs.setFigmaDialogOpen(true)}
              disabled={taskInputs.uploadFile.isPending}
            />
          </PopoverSelector>
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

      <FigmaImportDialog
        open={taskInputs.figmaDialogOpen}
        onOpenChange={taskInputs.setFigmaDialogOpen}
        onImportSuccess={taskInputs.handleFigmaImportSuccess}
      />
    </Card>
  )
}
