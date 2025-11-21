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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

interface CreateRepoDialogProps {
  isOpen: boolean
  onOpenChange: (open: boolean) => void
  defaultName: string
  onSubmit: (repositoryName: string, visibility: 'private' | 'public') => void
  isPending: boolean
}

// Sanitize name for GitHub repository naming requirements
function sanitizeRepoName(name: string): string {
  return name
    .replace(/[^a-zA-Z0-9._-]/g, '-') // Replace invalid chars with hyphens
    .replace(/-+/g, '-') // Collapse multiple hyphens
    .replace(/^-|-$/g, '') // Remove leading/trailing hyphens
    .slice(0, 100) // Max 100 chars
}

export function CreateRepoDialog({
  isOpen,
  onOpenChange,
  defaultName,
  onSubmit,
  isPending,
}: CreateRepoDialogProps) {
  const [repositoryName, setRepositoryName] = useState(sanitizeRepoName(defaultName))
  const [visibility, setVisibility] = useState<'private' | 'public'>('private')
  const [error, setError] = useState<string | null>(null)

  // Reset form when dialog opens
  const handleOpenChange = (open: boolean) => {
    if (open) {
      setRepositoryName(sanitizeRepoName(defaultName))
      setVisibility('private')
      setError(null)
    }
    onOpenChange(open)
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    // Validate repository name
    if (!repositoryName.trim()) {
      setError('Repository name is required')
      return
    }

    const nameRegex = /^[a-zA-Z0-9._-]+$/
    if (!nameRegex.test(repositoryName)) {
      setError(
        'Repository name can only contain letters, numbers, hyphens, underscores, and periods',
      )
      return
    }

    if (repositoryName.length > 100) {
      setError('Repository name must be less than 100 characters')
      return
    }

    setError(null)
    onSubmit(repositoryName, visibility)
  }

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Create GitHub Repository</DialogTitle>
            <DialogDescription>
              Create a new GitHub repository and push your code to it.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="repo-name">Repository Name</Label>
              <Input
                id="repo-name"
                value={repositoryName}
                onChange={(e) => {
                  setRepositoryName(e.target.value)
                  setError(null)
                }}
                placeholder="my-awesome-project"
                disabled={isPending}
              />
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>

            <div className="grid gap-2">
              <Label>Visibility</Label>
              <Select
                value={visibility}
                onValueChange={(value: 'private' | 'public') => setVisibility(value)}
                disabled={isPending}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select visibility" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="private">
                    Private - Only you can see this repository
                  </SelectItem>
                  <SelectItem value="public">Public - Anyone can see this repository</SelectItem>
                </SelectContent>
              </Select>
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
                  Creating...
                </>
              ) : (
                'Create Repository'
              )}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
