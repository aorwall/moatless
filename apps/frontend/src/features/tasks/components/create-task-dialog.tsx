import { useState } from 'react'
import { Dialog, DialogContent } from '@/lib/components/ui/dialog'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'
import { CreateCodeTask } from './create-task/create-code-task'

interface CreateTaskDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultRepositoryId?: string
  defaultMessage?: string
}

export function CreateTaskDialog({
  open,
  onOpenChange,
  defaultRepositoryId,
  defaultMessage,
}: CreateTaskDialogProps) {
  const [model, setModel] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)
  const [agentMode, setAgentMode] = useState<'edit' | 'plan'>('edit')
  const [repositoryId, setRepositoryId] = useState<string>(defaultRepositoryId || '')

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl md:max-w-3xl lg:max-w-4xl">
        <div className="space-y-4">
          <CreateCodeTask
            className="w-full"
            maxHeight="500px"
            defaultMessage={defaultMessage}
            model={model}
            onModelChange={setModel}
            agentMode={agentMode}
            onAgentModeChange={setAgentMode}
            repositoryId={repositoryId}
            onRepositoryChange={setRepositoryId}
            onSuccess={() => onOpenChange(false)}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}
