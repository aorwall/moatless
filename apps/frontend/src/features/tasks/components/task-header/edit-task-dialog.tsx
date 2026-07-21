import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'

interface EditTaskDialogProps {
  isOpen: boolean
  onOpenChange: (open: boolean) => void
  currentName: string
  currentDescription: string
  onSubmit: (updates: { name?: string; description?: string }) => void
  isPending: boolean
}

// Validate name for URL-friendly format (alphanumeric and dashes only)
function validateName(name: string): string | null {
  if (!name.trim()) {
    return null // Empty name is allowed (optional field)
  }
  const nameRegex = /^[a-zA-Z0-9-]+$/
  if (!nameRegex.test(name)) {
    return 'Name can only contain letters, numbers, and dashes'
  }
  if (name.length > 100) {
    return 'Name must be less than 100 characters'
  }
  return null
}

export function EditTaskDialog({
  isOpen,
  onOpenChange,
  currentName,
  currentDescription,
  onSubmit,
  isPending,
}: EditTaskDialogProps) {
  const [name, setName] = useState(currentName)
  const [description, setDescription] = useState(currentDescription)
  const [error, setError] = useState<string | null>(null)

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    // Validate name if provided
    const nameError = validateName(name)
    if (nameError) {
      setError(nameError)
      return
    }

    setError(null)

    // Build updates object with only changed values
    const updates: { name?: string; description?: string } = {}
    if (name !== currentName) {
      updates.name = name
    }
    if (description !== currentDescription) {
      updates.description = description
    }

    // Only submit if there are changes
    if (Object.keys(updates).length > 0) {
      onSubmit(updates)
    } else {
      onOpenChange(false)
    }
  }

  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Edit Task</DialogTitle>
            <DialogDescription>Update the task name and description.</DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="task-name">Name</Label>
              <Input
                id="task-name"
                value={name}
                onChange={(e) => {
                  setName(e.target.value)
                  setError(null)
                }}
                placeholder="my-task-name"
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                URL-friendly name (letters, numbers, and dashes only)
              </p>
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>

            <div className="grid gap-2">
              <Label htmlFor="task-description">Description</Label>
              <Textarea
                id="task-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="A brief description of the task"
                disabled={isPending}
                rows={3}
              />
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isPending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isPending}>
              {isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Saving...
                </>
              ) : (
                'Save'
              )}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
