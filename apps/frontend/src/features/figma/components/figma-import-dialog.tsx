import { useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2, Plug } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useFigmaConfig, useGetFigmaOAuthUrl } from '@/features/settings'
import { queryKeys } from '@/lib/api/query-keys'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
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
    hasImage: boolean
  }) => void
}

export function FigmaImportDialog({ open, onOpenChange, onImportSuccess }: FigmaImportDialogProps) {
  const [url, setUrl] = useState('')
  const [error, setError] = useState<string | null>(null)
  const queryClient = useQueryClient()

  // Figma config and OAuth
  const { data: figmaConfig, isLoading: configLoading } = useFigmaConfig()
  const { refetch: fetchOAuthUrl } = useGetFigmaOAuthUrl({ enabled: false })
  const [isConnecting, setIsConnecting] = useState(false)

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

  // Listen for window focus to detect when OAuth popup closes
  // biome-ignore lint/correctness/useExhaustiveDependencies: queryClient is stable
  useEffect(() => {
    if (!isConnecting) {
      return
    }

    // When OAuth popup is opened, listen for window focus to detect when user returns
    const handleFocus = () => {
      // Invalidate Figma config to refetch connection status
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.figma() })
    }

    window.addEventListener('focus', handleFocus)

    return () => {
      window.removeEventListener('focus', handleFocus)
    }
  }, [isConnecting, queryClient])

  // Reset connecting state and show success when connection is detected
  useEffect(() => {
    // When config updates and shows connected, reset connecting state
    if (isConnecting && figmaConfig?.connected) {
      setIsConnecting(false)
      toast.success('Successfully connected to Figma!')
    }
  }, [isConnecting, figmaConfig?.connected])

  const handleConnect = async () => {
    setIsConnecting(true)
    try {
      const result = await fetchOAuthUrl()
      if (result.data?.authUrl) {
        const width = 600
        const height = 700
        const left = window.screen.width / 2 - width / 2
        const top = window.screen.height / 2 - height / 2

        window.open(
          result.data.authUrl,
          'figma-oauth',
          `width=${width},height=${height},left=${left},top=${top}`,
        )

        toast.info('Continue in the new window to connect your Figma account')
        // Don't set isConnecting=false yet - keep it true while OAuth is in progress
        // The focus event will trigger refetch, and we'll set it to false when connection succeeds
      } else {
        setIsConnecting(false)
      }
    } catch (error) {
      toast.error('Failed to initiate Figma OAuth', {
        description: getErrorMessage(error),
      })
      setIsConnecting(false)
    }
  }

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
          {configLoading ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="h-8 w-8 animate-spin" />
            </div>
          ) : !figmaConfig?.connected ? (
            // Not connected - show connection UI
            <div className="grid gap-4 py-4">
              <Alert>
                <Plug className="h-4 w-4" />
                <AlertTitle>Connect to Figma</AlertTitle>
                <AlertDescription>
                  <p className="mb-4">
                    You need to connect your Figma account before importing designs.
                  </p>
                  <Button onClick={handleConnect} disabled={isConnecting} className="w-full">
                    {isConnecting ? (
                      <>
                        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                        Connecting...
                      </>
                    ) : (
                      <>
                        <Plug className="mr-2 h-4 w-4" />
                        Connect to Figma
                      </>
                    )}
                  </Button>
                </AlertDescription>
              </Alert>

              <Alert>
                <AlertTitle>How to connect</AlertTitle>
                <AlertDescription className="space-y-2">
                  <ol className="list-decimal list-inside space-y-1 text-sm">
                    <li>Click "Connect to Figma" above</li>
                    <li>Authorize the application in the popup window</li>
                    <li>Return here to import your design</li>
                  </ol>
                  <p className="text-sm text-muted-foreground mt-2">
                    To create a Figma OAuth app, visit{' '}
                    <a
                      href="https://www.figma.com/developers/apps"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-blue-600 hover:underline inline-flex items-center gap-1"
                    >
                      Figma Developer Apps
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  </p>
                </AlertDescription>
              </Alert>
            </div>
          ) : (
            // Connected - show import form
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
          )}

          <DialogFooter>
            {figmaConfig?.connected && (
              <>
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
              </>
            )}
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
