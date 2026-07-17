import { createLazyFileRoute } from '@tanstack/react-router'
import { Building2, Loader2, Plus, Save, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { AdminPage } from '@/features/admin'
import {
  useAdminTeamMembers,
  useAdminTeamsList,
  useAdminUsersList,
  useRemoveAdminTeamMember,
  useSaveAdminTeam,
  useSaveAdminTeamMember,
} from '@/features/admin/hooks/use-admin-users-teams'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

const TEAM_ROLES = ['member', 'manager', 'owner'] as const

export const Route = createLazyFileRoute('/admin/teams/')({
  component: AdminTeamsPage,
})

function AdminTeamsPage() {
  const teamsQuery = useAdminTeamsList()
  const usersQuery = useAdminUsersList()
  const teams = teamsQuery.data ?? []
  const users = usersQuery.data?.users ?? []
  const [selectedTeamId, setSelectedTeamId] = useState('')
  const selectedTeam = teams.find((team) => team.id === selectedTeamId) ?? null

  useEffect(() => {
    if (!selectedTeamId && teams[0]) {
      setSelectedTeamId(teams[0].id)
    }
  }, [selectedTeamId, teams])

  const membersQuery = useAdminTeamMembers(selectedTeamId, !!selectedTeamId)
  const members = membersQuery.data ?? []
  const memberIds = useMemo(() => new Set(members.map((member) => member.userId)), [members])
  const eligibleUsers = users.filter((user) => !memberIds.has(user.id))
  const error = teamsQuery.error ?? usersQuery.error ?? membersQuery.error
  const isLoading = teamsQuery.isLoading || usersQuery.isLoading

  return (
    <AdminPage
      title="Teams"
      description="Create teams, update team metadata, and manage team members and roles."
    >
      {isLoading ? (
        <div className="flex h-32 items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin" />
        </div>
      ) : error ? (
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>{getErrorMessage(error)}</AlertDescription>
        </Alert>
      ) : (
        <div className="grid gap-6 xl:grid-cols-[minmax(16rem,22rem)_1fr]">
          <section className="space-y-4">
            <NewTeamForm />
            <div className="rounded-md border">
              {teams.length === 0 ? (
                <div className="py-12 text-center text-muted-foreground">
                  <Building2 className="mx-auto mb-3 h-10 w-10 opacity-50" />
                  <p>No teams found</p>
                </div>
              ) : (
                teams.map((team) => (
                  <button
                    type="button"
                    key={team.id}
                    onClick={() => setSelectedTeamId(team.id)}
                    className={`block w-full border-b px-4 py-3 text-left last:border-b-0 hover:bg-muted ${
                      selectedTeamId === team.id ? 'bg-muted' : ''
                    }`}
                  >
                    <div className="font-medium">{team.name}</div>
                    <div className="text-xs text-muted-foreground">{team.slug}</div>
                  </button>
                ))
              )}
            </div>
          </section>

          {selectedTeam ? (
            <TeamDetail
              key={selectedTeam.id}
              team={selectedTeam}
              users={users}
              members={members}
              membersLoading={membersQuery.isLoading}
              eligibleUsers={eligibleUsers}
            />
          ) : (
            <div className="rounded-md border border-dashed py-12 text-center text-sm text-muted-foreground">
              Select a team to manage members.
            </div>
          )}
        </div>
      )}
    </AdminPage>
  )
}

function NewTeamForm() {
  const saveTeam = useSaveAdminTeam()
  const [name, setName] = useState('')
  const [slug, setSlug] = useState('')

  const handleCreate = () => {
    saveTeam.mutate(
      { request: { name: name.trim(), slug: slug.trim(), description: null } },
      {
        onSuccess: () => {
          toast.success('Team created')
          setName('')
          setSlug('')
        },
        onError: (error) =>
          toast.error('Failed to create team', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <div className="space-y-3 rounded-md border p-4">
      <div className="text-sm font-medium">New Team</div>
      <Input value={name} onChange={(event) => setName(event.target.value)} placeholder="Name" />
      <Input value={slug} onChange={(event) => setSlug(event.target.value)} placeholder="slug" />
      <Button
        size="sm"
        onClick={handleCreate}
        disabled={saveTeam.isPending || !name.trim() || !slug.trim()}
      >
        <Plus className="mr-2 h-4 w-4" />
        Create
      </Button>
    </div>
  )
}

function TeamDetail({
  team,
  users,
  members,
  membersLoading,
  eligibleUsers,
}: {
  team: {
    id: string
    name: string
    slug: string
    description?: string | null
  }
  users: Array<{ id: string; login: string; name?: string | null }>
  members: Array<{ teamId: string; userId: string; role: string; createdAt: string }>
  membersLoading: boolean
  eligibleUsers: Array<{ id: string; login: string; name?: string | null }>
}) {
  const saveTeam = useSaveAdminTeam()
  const saveMember = useSaveAdminTeamMember()
  const removeMember = useRemoveAdminTeamMember()
  const [name, setName] = useState(team.name)
  const [description, setDescription] = useState(team.description ?? '')
  const [newUserId, setNewUserId] = useState('')
  const [newRole, setNewRole] = useState<(typeof TEAM_ROLES)[number]>('member')

  useEffect(() => {
    setName(team.name)
    setDescription(team.description ?? '')
    setNewUserId('')
    setNewRole('member')
  }, [team.name, team.description])

  const usersById = useMemo(() => new Map(users.map((user) => [user.id, user])), [users])
  const teamChanged = name !== team.name || description !== (team.description ?? '')

  const handleSaveTeam = () => {
    saveTeam.mutate(
      { teamId: team.id, request: { name: name.trim(), description: description.trim() || null } },
      {
        onSuccess: () => toast.success('Team updated'),
        onError: (error) =>
          toast.error('Failed to update team', { description: getErrorMessage(error) }),
      },
    )
  }

  const handleAddMember = () => {
    saveMember.mutate(
      { teamId: team.id, userId: newUserId, role: newRole },
      {
        onSuccess: () => {
          toast.success('Member added')
          setNewUserId('')
          setNewRole('member')
        },
        onError: (error) =>
          toast.error('Failed to add member', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <section className="space-y-6">
      <div className="grid gap-4 rounded-md border p-4 md:grid-cols-[1fr_1fr_auto]">
        <div className="space-y-2">
          <Label>Name</Label>
          <Input value={name} onChange={(event) => setName(event.target.value)} />
        </div>
        <div className="space-y-2">
          <Label>Slug</Label>
          <Input value={team.slug} disabled />
        </div>
        <div className="flex items-end">
          <Button
            onClick={handleSaveTeam}
            disabled={saveTeam.isPending || !teamChanged || !name.trim()}
          >
            {saveTeam.isPending ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Save className="mr-2 h-4 w-4" />
            )}
            Save
          </Button>
        </div>
        <div className="space-y-2 md:col-span-3">
          <Label>Description</Label>
          <Textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            className="min-h-20"
          />
        </div>
      </div>

      <div className="flex flex-wrap items-end gap-3 rounded-md border p-4">
        <div className="min-w-64 flex-1 space-y-2">
          <Label>Add User</Label>
          <Select value={newUserId} onValueChange={setNewUserId}>
            <SelectTrigger>
              <SelectValue placeholder="Select user" />
            </SelectTrigger>
            <SelectContent>
              {eligibleUsers.length === 0 ? (
                <SelectItem value="__none__" disabled>
                  No users available
                </SelectItem>
              ) : (
                eligibleUsers.map((user) => (
                  <SelectItem key={user.id} value={user.id}>
                    {user.login}
                  </SelectItem>
                ))
              )}
            </SelectContent>
          </Select>
        </div>
        <div className="w-40 space-y-2">
          <Label>Role</Label>
          <Select value={newRole} onValueChange={(value) => setNewRole(value as typeof newRole)}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TEAM_ROLES.map((role) => (
                <SelectItem key={role} value={role}>
                  {role}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <Button onClick={handleAddMember} disabled={saveMember.isPending || !newUserId}>
          <Plus className="mr-2 h-4 w-4" />
          Add
        </Button>
      </div>

      {membersLoading ? (
        <div className="flex h-24 items-center justify-center rounded-md border">
          <Loader2 className="h-6 w-6 animate-spin" />
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>User</TableHead>
              <TableHead>Role</TableHead>
              <TableHead>Joined</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {members.map((member) => {
              const user = usersById.get(member.userId)
              return (
                <TableRow key={member.userId}>
                  <TableCell>
                    <div className="font-medium">{user?.login ?? member.userId}</div>
                    {user?.name && <div className="text-xs text-muted-foreground">{user.name}</div>}
                  </TableCell>
                  <TableCell>
                    <Select
                      value={member.role}
                      onValueChange={(role) =>
                        saveMember.mutate(
                          { teamId: team.id, userId: member.userId, role },
                          {
                            onSuccess: () => toast.success('Member role updated'),
                            onError: (error) =>
                              toast.error('Failed to update member', {
                                description: getErrorMessage(error),
                              }),
                          },
                        )
                      }
                    >
                      <SelectTrigger className="w-36">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {TEAM_ROLES.map((role) => (
                          <SelectItem key={role} value={role}>
                            {role}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </TableCell>
                  <TableCell>
                    <Badge variant="outline">
                      {new Date(member.createdAt).toLocaleDateString()}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() =>
                        removeMember.mutate(
                          { teamId: team.id, userId: member.userId },
                          {
                            onSuccess: () => toast.success('Member removed'),
                            onError: (error) =>
                              toast.error('Failed to remove member', {
                                description: getErrorMessage(error),
                              }),
                          },
                        )
                      }
                      disabled={removeMember.isPending}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}
    </section>
  )
}
