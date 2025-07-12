import { useState } from 'react'

import { ViewToggle } from '@/components/view-toggle'
import { Dialog, DialogContent } from '@/lib/components/ui/dialog'
import { CreateCodeTask } from './create-task/create-code-task'
import { CreateVibeTask } from './create-task/create-vibe-task'

type ViewMode = 'code' | 'vibe'

interface CreateTaskDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultRepositoryId?: string
  defaultMessage?: string
  defaultMode?: ViewMode
}

export function CreateTaskDialog({
  open,
  onOpenChange,
  defaultRepositoryId,
  defaultMessage,
  defaultMode = 'code',
}: CreateTaskDialogProps) {
  const [mode, setMode] = useState<ViewMode>(defaultMode)

  const handleModeChange = (newMode: ViewMode) => {
    setMode(newMode)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl md:max-w-3xl lg:max-w-4xl">
        <div className="space-y-4">
          {/* View Toggle */}
          <div className="flex justify-center">
            <ViewToggle mode={mode} onModeChange={handleModeChange} />
          </div>

          {/* Task Creation Components */}
          {mode === 'code' ? (
            <CreateCodeTask
              className="w-full"
              maxHeight="500px"
              defaultRepositoryId={defaultRepositoryId}
              defaultMessage={defaultMessage}
            />
          ) : (
            <CreateVibeTask className="w-full" maxHeight="500px" defaultMessage={defaultMessage} />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
