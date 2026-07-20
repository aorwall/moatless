import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useAddMarketplace } from '@/features/plugins/hooks/use-plugins'
import { ConfigurationInput } from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Label } from '@/lib/components/ui/label'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createLazyFileRoute('/admin/marketplaces/new')({
  component: NewMarketplacePage,
})

function NewMarketplacePage() {
  const navigate = useNavigate()
  const addMarketplace = useAddMarketplace()
  const [repoUrl, setRepoUrl] = useState('')

  const handleAdd = () => {
    const trimmed = repoUrl.trim()
    if (!trimmed) {
      toast.error('Repository URL is required')
      return
    }
    addMarketplace.mutate(
      { repoUrl: trimmed },
      {
        onSuccess: async (marketplace) => {
          toast.success('Marketplace added successfully')
          await navigate({
            to: '/admin/marketplaces/$marketplace-id',
            params: { 'marketplace-id': marketplace.id },
          })
        },
        onError: (error) =>
          toast.error('Failed to add marketplace', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">Add marketplace</h1>
        <p className="text-sm text-muted-foreground">
          Add a Claude Code plugin marketplace from a GitHub repository.
        </p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>New Marketplace</CardTitle>
          <CardDescription>
            A public GitHub repository containing Claude Code plugins.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="repo-url">GitHub Repository URL</Label>
            <ConfigurationInput
              id="repo-url"
              type="text"
              placeholder="https://github.com/owner/repo"
              value={repoUrl}
              onChange={(e) => setRepoUrl(e.target.value)}
            />
          </div>
          <Button onClick={handleAdd} disabled={addMarketplace.isPending || !repoUrl.trim()}>
            {addMarketplace.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            Add Marketplace
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
