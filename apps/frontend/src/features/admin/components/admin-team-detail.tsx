import { Loader2, Plus, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import type {
  AdminTeam,
  AdminTeamMember,
  AdminUserListItem,
} from '@/features/admin/hooks/use-admin-users-teams'
import {
  useAdminTeamMembers,
  useAdminTeamsList,
  useAdminUsersList,
  useRemoveAdminTeamMember,
  useSaveAdminTeam,
  useSaveAdminTeamMember,
} from '@/features/admin/hooks/use-admin-users-teams'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'
import { EditFormCard } from '@/lib/components/ui/edit-form-card'
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
import { useDirtyForm } from '@/lib/hooks/use-dirty-form'
import { getErrorMessage } from '@/lib/utils/error'

const TEAM_ROLES = ['member', 'manager', 'owner'] as const

export function AdminTeamDetail({ teamId }: { teamId: string }) {
  const teamsQuery = useAdminTeamsList()
  const team = teamsQuery.data?.find((candidate) => candidate.id === teamId)

  if (teamsQuery.isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!team) {
    return <DetailNotFound title="Team not found" description="This team may have been deleted." />
  }

  return <AdminTeamDetailBody key={team.id} team={team} />
}

function AdminTeamDetailBody({ team }: { team: AdminTeam }) {
  const usersQuery = useAdminUsersList()
  const membersQuery = useAdminTeamMembers(team.id, true)
  const saveTeam = useSaveAdminTeam()

  const users = usersQuery.data?.users ?? []
  const members = membersQuery.data ?? []

  const form = useDirtyForm({ name: team.name, description: team.description ?? '' })

  const handleSaveTeam = () => {
    saveTeam.mutate(
      {
        teamId: team.id,
        request: {
          name: form.values.name.trim(),
          description: form.values.description.trim() || null,
        },
      },
      {
        onSuccess: () => toast.success('Team updated'),
        onError: (error) =>
          toast.error('Failed to update team', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <DetailHeader
        title={team.name}
        subtitle={team.slug}
        badges={team.isDefault ? <Badge variant="secondary">default</Badge> : null}
      />

      <EditFormCard
        title="Team"
        description="Update the team name and description."
        isDirty={form.isDirty && form.values.name.trim().length > 0}
        isPending={saveTeam.isPending}
        onSave={handleSaveTeam}
        onReset={form.reset}
      >
        <div className="space-y-2">
          <Label htmlFor="team-name">Name</Label>
          <Input
            id="team-name"
            value={form.values.name}
            onChange={(event) => form.setField('name', event.target.value)}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="team-slug">Slug</Label>
          <Input id="team-slug" value={team.slug} disabled />
        </div>
        <div className="space-y-2">
          <Label htmlFor="team-description">Description</Label>
          <Textarea
            id="team-description"
            value={form.values.description}
            onChange={(event) => form.setField('description', event.target.value)}
            className="min-h-20"
          />
        </div>
      </EditFormCard>

      <TeamMembersCard
        teamId={team.id}
        users={users}
        members={members}
        membersLoading={membersQuery.isLoading}
      />
    </div>
  )
}

function TeamMembersCard({
  teamId,
  users,
  members,
  membersLoading,
}: {
  teamId: string
  users: AdminUserListItem[]
  members: AdminTeamMember[]
  membersLoading: boolean
}) {
  const saveMember = useSaveAdminTeamMember()
  const removeMember = useRemoveAdminTeamMember()
  const [newUserId, setNewUserId] = useState('')
  const [newRole, setNewRole] = useState<(typeof TEAM_ROLES)[number]>('member')

  const usersById = useMemo(() => new Map(users.map((user) => [user.id, user])), [users])
  const memberIds = useMemo(() => new Set(members.map((member) => member.userId)), [members])
  const eligibleUsers = users.filter((user) => !memberIds.has(user.id))

  const handleAddMember = () => {
    saveMember.mutate(
      { teamId, userId: newUserId, role: newRole },
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
    <Card>
      <CardHeader>
        <CardTitle>Members</CardTitle>
        <CardDescription>Manage team members and their roles.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-56 flex-1 space-y-2">
            <Label>Add user</Label>
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
        ) : members.length === 0 ? (
          <p className="text-sm text-muted-foreground">No members yet.</p>
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
                      {user?.name ? (
                        <div className="text-xs text-muted-foreground">{user.name}</div>
                      ) : null}
                    </TableCell>
                    <TableCell>
                      <Select
                        value={member.role}
                        onValueChange={(role) =>
                          saveMember.mutate(
                            { teamId, userId: member.userId, role },
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
                            { teamId, userId: member.userId },
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
      </CardContent>
    </Card>
  )
}
