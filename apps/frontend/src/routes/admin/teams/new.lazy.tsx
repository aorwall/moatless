import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'
import { useSaveAdminTeam } from '@/features/admin/hooks/use-admin-users-teams'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createLazyFileRoute('/admin/teams/new')({
  component: NewTeamPage,
})

function NewTeamPage() {
  const navigate = useNavigate()
  const saveTeam = useSaveAdminTeam()
  const [name, setName] = useState('')
  const [slug, setSlug] = useState('')
  const [description, setDescription] = useState('')

  const handleCreate = () => {
    saveTeam.mutate(
      {
        request: { name: name.trim(), slug: slug.trim(), description: description.trim() || null },
      },
      {
        onSuccess: async () => {
          toast.success('Team created')
          await navigate({ to: '/admin/teams' })
        },
        onError: (error) =>
          toast.error('Failed to create team', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">New team</h1>
        <p className="text-sm text-muted-foreground">Create a team and manage its members.</p>
      </div>
      <Card>
        <CardHeader>
          <CardTitle>Team</CardTitle>
          <CardDescription>Name and slug are required.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="new-team-name">Name</Label>
            <Input
              id="new-team-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Team name"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="new-team-slug">Slug</Label>
            <Input
              id="new-team-slug"
              value={slug}
              onChange={(event) => setSlug(event.target.value)}
              placeholder="team-slug"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="new-team-description">Description</Label>
            <Textarea
              id="new-team-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              className="min-h-20"
            />
          </div>
          <Button
            onClick={handleCreate}
            disabled={saveTeam.isPending || !name.trim() || !slug.trim()}
          >
            Create team
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
