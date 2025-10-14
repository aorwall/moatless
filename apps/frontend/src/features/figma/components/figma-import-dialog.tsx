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
import { getErrorMessage } from '@/lib/utils/error'
import { useFigmaImport } from '../hooks/use-figma-import'

interface FigmaImportDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImportSuccess?: (data: {
    importId: string
    fileId: string
    fileName: string
    nodeName: string
  }) => void
}

export function FigmaImportDialog({ open, onOpenChange, onImportSuccess }: FigmaImportDialogProps) {
  const [url, setUrl] = useState('')
  const [error, setError] = useState<string | null>(null)

  const importMutation = useFigmaImport({
    onSuccess: (data) => {
      // Clear form and close dialog
      setUrl('')
      setError(null)
      onOpenChange(false)
      onImportSuccess?.(data)
    },
    onError: (error) => {
      setError(getErrorMessage(error))
    },
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)

    if (!url.trim()) {
      setError('Please enter a Figma URL')
      return
    }

    // Validate URL format
    if (!url.includes('figma.com')) {
      setError('Please enter a valid Figma URL')
      return
    }

    // Check for node-id parameter
    if (!url.includes('node-id=')) {
      setError('Figma URL must include a node-id parameter')
      return
    }

    importMutation.mutate(url.trim())
  }

  const handleOpenChange = (newOpen: boolean) => {
    if (!(newOpen || importMutation.isPending)) {
      setUrl('')
      setError(null)
    }
    onOpenChange(newOpen)
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>Import from Figma</DialogTitle>
          <DialogDescription>
            Paste a Figma URL with a node-id parameter to import the design.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit}>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="figma-url">Figma URL</Label>
              <Input
                id="figma-url"
                placeholder="https://www.figma.com/design/...?node-id=..."
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                disabled={importMutation.isPending}
              />
              {error && <p className="text-sm text-destructive">{error}</p>}
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => handleOpenChange(false)}
              disabled={importMutation.isPending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={importMutation.isPending}>
              {importMutation.isPending ? 'Importing...' : 'Import'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
