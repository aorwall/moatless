import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { RepositoryConfigCreateSection } from './sections/repository-config-create-section'

interface CreateRepositoryDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Called with the new repository id after a successful create. */
  onCreated?: (repositoryId: string) => void
}

/**
 * Lightweight create-only repository dialog used inline in task-creation flows.
 * Full repository management lives at the `/admin/repositories` route.
 */
export function CreateRepositoryDialog({
  open,
  onOpenChange,
  onCreated,
}: CreateRepositoryDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add repository</DialogTitle>
          <DialogDescription>
            Connect a Git repository to use as a starting point for your task.
          </DialogDescription>
        </DialogHeader>
        <RepositoryConfigCreateSection
          onCancel={() => onOpenChange(false)}
          onSuccess={(repositoryId) => {
            onCreated?.(repositoryId)
            onOpenChange(false)
          }}
        />
      </DialogContent>
    </Dialog>
  )
}
