import { Check, Copy, Key, Loader2, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  type ApiKey,
  type ApiKeyCreateResult,
  useApiKeys,
  useCreateApiKey,
  useRevokeApiKey,
} from '@/features/settings/hooks/use-api-keys'
import { ConfigurationGroup, ConfigurationSectionWrapper } from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/lib/components/ui/alert-dialog'
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
import { getErrorMessage } from '@/lib/utils/error'

export function ApiKeyConfigurationSection() {
  const { data: apiKeys, isLoading, error } = useApiKeys()
  const createApiKey = useCreateApiKey()
  const revokeApiKey = useRevokeApiKey()

  const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false)
  const [isSuccessDialogOpen, setIsSuccessDialogOpen] = useState(false)
  const [createdKey, setCreatedKey] = useState<ApiKeyCreateResult | null>(null)
  const [keyToRevoke, setKeyToRevoke] = useState<ApiKey | null>(null)
  const [newKeyName, setNewKeyName] = useState('')
  const [expiresInDays, setExpiresInDays] = useState<string>('never')
  const [copied, setCopied] = useState(false)

  const handleCreate = () => {
    if (!newKeyName.trim()) {
      toast.error('Please enter a name for the API key')
      return
    }

    const request = {
      name: newKeyName.trim(),
      expiresInDays: expiresInDays === 'never' ? undefined : Number.parseInt(expiresInDays, 10),
    }

    createApiKey.mutate(request, {
      onSuccess: (result) => {
        setCreatedKey(result)
        setIsCreateDialogOpen(false)
        setIsSuccessDialogOpen(true)
        setNewKeyName('')
        setExpiresInDays('never')
        toast.success('API key created successfully')
      },
      onError: (error) => {
        toast.error('Failed to create API key', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleRevoke = (key: ApiKey) => {
    setKeyToRevoke(key)
  }

  const confirmRevoke = () => {
    if (!keyToRevoke) {
      return
    }

    revokeApiKey.mutate(keyToRevoke.id, {
      onSuccess: () => {
        toast.success('API key revoked successfully')
        setKeyToRevoke(null)
      },
      onError: (error) => {
        toast.error('Failed to revoke API key', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      toast.success('API key copied to clipboard')
      setTimeout(() => setCopied(false), 2000)
    } catch {
      toast.error('Failed to copy to clipboard')
    }
  }

  const formatDate = (dateString: string | null | undefined) => {
    if (!dateString) {
      return 'Never'
    }
    return new Date(dateString).toLocaleDateString()
  }

  const formatRelativeTime = (dateString: string | null | undefined) => {
    if (!dateString) {
      return 'Never used'
    }
    const date = new Date(dateString)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24))

    if (diffDays === 0) {
      return 'Today'
    }
    if (diffDays === 1) {
      return 'Yesterday'
    }
    if (diffDays < 7) {
      return `${diffDays} days ago`
    }
    if (diffDays < 30) {
      return `${Math.floor(diffDays / 7)} weeks ago`
    }
    return formatDate(dateString)
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load API keys. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  const keys = apiKeys ?? []

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="API Keys" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Create API keys for programmatic access to the API. API keys can be used instead of
          session authentication for automation and integrations.
        </p>

        {/* Create Button */}
        <div className="flex justify-end mb-4">
          <Button onClick={() => setIsCreateDialogOpen(true)} size="sm">
            <Plus className="mr-2 h-4 w-4" />
            Create API Key
          </Button>
        </div>

        {/* API Keys List */}
        {keys.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">
            <Key className="h-12 w-12 mx-auto mb-4 opacity-50" />
            <p>No API keys yet</p>
            <p className="text-sm">Create an API key to get started</p>
          </div>
        ) : (
          <div className="space-y-3">
            {keys.map((key) => (
              <div
                key={key.id}
                className="flex items-center justify-between p-4 border rounded-lg bg-card"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <Key className="h-4 w-4 text-muted-foreground shrink-0" />
                    <span className="font-medium truncate">{key.name}</span>
                  </div>
                  <div className="mt-1 text-sm text-muted-foreground space-y-0.5">
                    <div className="font-mono text-xs">{key.keyPrefix}...</div>
                    <div className="flex gap-4 text-xs">
                      <span>Created: {formatDate(key.createdAt)}</span>
                      <span>Last used: {formatRelativeTime(key.lastUsedAt)}</span>
                      {key.expiresAt && <span>Expires: {formatDate(key.expiresAt)}</span>}
                    </div>
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleRevoke(key)}
                  className="text-destructive hover:text-destructive hover:bg-destructive/10 shrink-0"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            ))}
          </div>
        )}
      </ConfigurationGroup>

      {/* Create API Key Dialog */}
      <Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create API Key</DialogTitle>
            <DialogDescription>
              Create a new API key for programmatic access. The key will only be shown once after
              creation.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label htmlFor="keyName">Name</Label>
              <Input
                id="keyName"
                placeholder="e.g., CI/CD Pipeline, E2E Tests"
                value={newKeyName}
                onChange={(e) => setNewKeyName(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="expiration">Expiration</Label>
              <Select value={expiresInDays} onValueChange={setExpiresInDays}>
                <SelectTrigger>
                  <SelectValue placeholder="Select expiration" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="never">Never expires</SelectItem>
                  <SelectItem value="7">7 days</SelectItem>
                  <SelectItem value="30">30 days</SelectItem>
                  <SelectItem value="90">90 days</SelectItem>
                  <SelectItem value="365">1 year</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setIsCreateDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleCreate} disabled={createApiKey.isPending || !newKeyName.trim()}>
              {createApiKey.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Key className="mr-2 h-4 w-4" />
              )}
              Create Key
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Success Dialog - Shows the created key */}
      <Dialog open={isSuccessDialogOpen} onOpenChange={setIsSuccessDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>API Key Created</DialogTitle>
            <DialogDescription>
              Your API key has been created. Make sure to copy it now - you won't be able to see it
              again!
            </DialogDescription>
          </DialogHeader>
          <div className="py-4">
            <Alert>
              <Key className="h-4 w-4" />
              <AlertTitle>Your API Key</AlertTitle>
              <AlertDescription>
                <div className="mt-2 flex items-center gap-2">
                  <code className="flex-1 p-2 bg-muted rounded font-mono text-sm break-all">
                    {createdKey?.key}
                  </code>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => createdKey && copyToClipboard(createdKey.key)}
                    className="shrink-0"
                  >
                    {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                  </Button>
                </div>
              </AlertDescription>
            </Alert>
            <p className="mt-4 text-sm text-muted-foreground">
              Use this key in the <code className="text-xs">Authorization</code> header:
              <br />
              <code className="text-xs bg-muted px-1 py-0.5 rounded">
                Authorization: Bearer {createdKey?.keyPrefix}...
              </code>
            </p>
          </div>
          <DialogFooter>
            <Button onClick={() => setIsSuccessDialogOpen(false)}>Done</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Revoke Confirmation Dialog */}
      <AlertDialog open={!!keyToRevoke} onOpenChange={() => setKeyToRevoke(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke API Key</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to revoke the API key "{keyToRevoke?.name}"? This action cannot
              be undone and any applications using this key will lose access immediately.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmRevoke}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {revokeApiKey.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Trash2 className="mr-2 h-4 w-4" />
              )}
              Revoke Key
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ConfigurationSectionWrapper>
  )
}
