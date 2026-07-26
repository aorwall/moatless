import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreatePlugin } from '@/features/plugins/hooks/use-plugins'
import { ConfigurationInput } from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Label } from '@/lib/components/ui/label'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createLazyFileRoute('/admin/plugins/new')({
  component: NewPluginPage,
})

function NewPluginPage() {
  const navigate = useNavigate()
  const createPlugin = useCreatePlugin()
  const [name, setName] = useState('')
  const [gitUrl, setGitUrl] = useState('')

  const trimmedName = name.trim()
  const trimmedGitUrl = gitUrl.trim()
  const canSubmit = trimmedName.length > 0 && trimmedGitUrl.length > 0

  const handleAdd = () => {
    if (!canSubmit) {
      toast.error('Name and git URL are both required')
      return
    }
    createPlugin.mutate(
      { name: trimmedName, gitUrl: trimmedGitUrl },
      {
        onSuccess: async (plugin) => {
          toast.success('Plugin registered')
          await navigate({
            to: '/admin/plugins/$plugin-id',
            params: { 'plugin-id': plugin.id },
          })
        },
        onError: (error) =>
          toast.error('Failed to register plugin', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">Register plugin</h1>
        <p className="text-sm text-muted-foreground">
          Register a plugin from a git repository so it can be used in sandboxes.
        </p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>New plugin</CardTitle>
          <CardDescription>A git repository containing a plugin.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="plugin-name">Name</Label>
            <ConfigurationInput
              id="plugin-name"
              type="text"
              placeholder="moatless-skills"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="git-url">Git URL</Label>
            <ConfigurationInput
              id="git-url"
              type="text"
              placeholder="https://github.com/owner/repo"
              value={gitUrl}
              onChange={(e) => setGitUrl(e.target.value)}
            />
          </div>
          <Button onClick={handleAdd} disabled={createPlugin.isPending || !canSubmit}>
            {createPlugin.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
            Register plugin
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
