import { UserRole } from '@moatless/api-client'
import { Bot, Loader2, Save } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useAdminTeamMembersByTeam,
  useAdminTeamsList,
  useAdminUser,
  useUpdateAdminUser,
} from '@/features/admin/hooks/use-admin-users-teams'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
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

type UserRoleValue = (typeof UserRole)[keyof typeof UserRole]

function formatDate(value: string | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

function ReadOnlyField({ label, value }: { label: string; value: string }) {
  return (
    <div className="space-y-1">
      <p className="text-xs font-medium text-muted-foreground">{label}</p>
      <p className="break-all text-sm">{value || '-'}</p>
    </div>
  )
}

export function AdminUserProfile({ login }: { login: string }) {
  const { user, isLoading, error } = useAdminUser(login)

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (error) {
    return <p className="p-6 text-sm text-destructive">{getErrorMessage(error)}</p>
  }

  if (!user) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-1 text-center text-muted-foreground">
        <p className="font-medium">User not found</p>
        <p className="text-sm">No user with login “{login}”.</p>
      </div>
    )
  }

  return <AdminUserProfileForm key={user.id} user={user} />
}

function AdminUserProfileForm({
  user,
}: {
  user: NonNullable<ReturnType<typeof useAdminUser>['user']>
}) {
  const updateUser = useUpdateAdminUser()
  const teamsQuery = useAdminTeamsList()
  const teams = teamsQuery.data ?? []
  const memberQueries = useAdminTeamMembersByTeam(teams)

  const memberships = useMemo(() => {
    const result: { teamId: string; teamName: string; role: string }[] = []
    for (const [index, team] of teams.entries()) {
      const members = memberQueries[index]?.data ?? []
      const membership = members.find((member) => member.userId === user.id)
      if (membership) {
        result.push({ teamId: team.id, teamName: team.name, role: membership.role })
      }
    }
    return result
  }, [teams, memberQueries, user.id])

  const [name, setName] = useState(user.name ?? '')
  const [email, setEmail] = useState(user.email ?? '')
  const [role, setRole] = useState<UserRoleValue>(user.role ?? UserRole.user)

  const isDirty =
    name !== (user.name ?? '') ||
    email !== (user.email ?? '') ||
    role !== (user.role ?? UserRole.user)

  const handleSave = () => {
    updateUser.mutate(
      {
        login: user.login,
        request: {
          name: name.trim() || null,
          email: email.trim() || null,
          role,
        },
      },
      {
        onSuccess: () => toast.success('User updated'),
        onError: (mutationError) =>
          toast.error('Failed to update user', {
            description: getErrorMessage(mutationError),
          }),
      },
    )
  }

  const handleReset = () => {
    setName(user.name ?? '')
    setEmail(user.email ?? '')
    setRole(user.role ?? UserRole.user)
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <div className="flex items-center gap-4">
        <span className="flex h-14 w-14 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted text-lg font-medium text-muted-foreground">
          {user.avatarUrl ? (
            <img src={user.avatarUrl} alt="" className="h-full w-full object-cover" />
          ) : (
            (user.name?.trim() || user.login).slice(0, 2).toUpperCase()
          )}
        </span>
        <div className="min-w-0 space-y-1">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-2xl font-semibold">{user.name || user.login}</h1>
            {user.isBot ? (
              <Badge variant="secondary" className="gap-1">
                <Bot className="h-3 w-3" /> bot
              </Badge>
            ) : null}
          </div>
          <p className="text-sm text-muted-foreground">{user.login}</p>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Profile</CardTitle>
          <CardDescription>Update the user's profile details and global role.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="user-name">Name</Label>
            <Input
              id="user-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Display name"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="user-email">Email</Label>
            <Input
              id="user-email"
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="user@example.com"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="user-role">Global role</Label>
            <Select value={role} onValueChange={(value) => setRole(value as UserRoleValue)}>
              <SelectTrigger id="user-role" className="w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={UserRole.user}>User</SelectItem>
                <SelectItem value={UserRole.admin}>Admin</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              Admins can access this admin area and manage all users and settings.
            </p>
          </div>
          <div className="flex items-center gap-2 pt-2">
            <Button onClick={handleSave} disabled={!isDirty || updateUser.isPending}>
              {updateUser.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <Save className="mr-2 h-4 w-4" />
              )}
              Save changes
            </Button>
            <Button
              variant="ghost"
              onClick={handleReset}
              disabled={!isDirty || updateUser.isPending}
            >
              Reset
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Account</CardTitle>
          <CardDescription>Identity and audit details (read-only).</CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-4">
          <ReadOnlyField label="User ID" value={user.id} />
          <ReadOnlyField label="Login" value={user.login} />
          <ReadOnlyField label="Provider" value={user.provider} />
          <ReadOnlyField label="Account type" value={user.isBot ? 'Bot' : 'User'} />
          <ReadOnlyField label="Created" value={formatDate(user.createdAt)} />
          <ReadOnlyField label="Last login" value={formatDate(user.lastLoginAt)} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Teams</CardTitle>
          <CardDescription>Team memberships for this user.</CardDescription>
        </CardHeader>
        <CardContent>
          {teamsQuery.isLoading || memberQueries.some((query) => query.isLoading) ? (
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          ) : memberships.length === 0 ? (
            <p className="text-sm text-muted-foreground">Not a member of any team.</p>
          ) : (
            <div className="flex flex-wrap gap-2">
              {memberships.map((membership) => (
                <Badge key={membership.teamId} variant="outline">
                  {membership.teamName}: {membership.role}
                </Badge>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
