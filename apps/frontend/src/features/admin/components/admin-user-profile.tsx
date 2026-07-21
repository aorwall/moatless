import { UserRole } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { adminResetPasswordHandler } from '@moatless/api-client/sandbox/users/users'
import { useMutation } from '@tanstack/react-query'
import { Bot, Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  useAdminTeamMembersByTeam,
  useAdminTeamsList,
  useAdminUser,
  useUpdateAdminUser,
} from '@/features/admin/hooks/use-admin-users-teams'
import { useAuthMode } from '@/features/auth/hooks/use-auth-mode'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailItem } from '@/lib/components/ui/detail-item'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'
import { EditFormCard } from '@/lib/components/ui/edit-form-card'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { ListAvatar } from '@/lib/components/ui/list-avatar'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { useDirtyForm } from '@/lib/hooks/use-dirty-form'
import { getErrorMessage } from '@/lib/utils/error'

type UserRoleValue = (typeof UserRole)[keyof typeof UserRole]

/** Mirrors the server's minimum in `validate_password`. */
const MIN_PASSWORD_LENGTH = 12

function formatDate(value: string | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
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
    return <DetailNotFound title="User not found" description={`No user with login “${login}”.`} />
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

  const memberships: { teamId: string; teamName: string; role: string }[] = []
  for (const [index, team] of teams.entries()) {
    const members = memberQueries[index]?.data ?? []
    const membership = members.find((member) => member.userId === user.id)
    if (membership) {
      memberships.push({ teamId: team.id, teamName: team.name, role: membership.role })
    }
  }

  const form = useDirtyForm<{ name: string; email: string; role: UserRoleValue }>({
    name: user.name ?? '',
    email: user.email ?? '',
    role: user.role ?? UserRole.user,
  })

  const handleSave = () => {
    updateUser.mutate(
      {
        login: user.login,
        request: {
          name: form.values.name.trim() || null,
          email: form.values.email.trim() || null,
          role: form.values.role,
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

  return (
    <div className="mx-auto max-w-2xl space-y-6 p-6">
      <DetailHeader
        avatar={
          <ListAvatar src={user.avatarUrl} label={user.name?.trim() || user.login} size="lg" />
        }
        title={user.name || user.login}
        subtitle={user.login}
        badges={
          user.isBot ? (
            <Badge variant="secondary" className="gap-1">
              <Bot className="h-3 w-3" /> bot
            </Badge>
          ) : null
        }
      />

      <EditFormCard
        title="Profile"
        description="Update the user's profile details and global role."
        isDirty={form.isDirty}
        isPending={updateUser.isPending}
        onSave={handleSave}
        onReset={form.reset}
      >
        <div className="space-y-2">
          <Label htmlFor="user-name">Name</Label>
          <Input
            id="user-name"
            value={form.values.name}
            onChange={(event) => form.setField('name', event.target.value)}
            placeholder="Display name"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="user-email">Email</Label>
          <Input
            id="user-email"
            type="email"
            value={form.values.email}
            onChange={(event) => form.setField('email', event.target.value)}
            placeholder="user@example.com"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="user-role">Global role</Label>
          <Select
            value={form.values.role}
            onValueChange={(value) => form.setField('role', value as UserRoleValue)}
          >
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
      </EditFormCard>

      <Card>
        <CardHeader>
          <CardTitle>Account</CardTitle>
          <CardDescription>Identity and audit details (read-only).</CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-4">
          <DetailItem label="User ID" value={<span className="break-all">{user.id}</span>} />
          <DetailItem label="Login" value={<span className="break-all">{user.login}</span>} />
          <DetailItem label="Provider" value={user.provider} />
          <DetailItem label="Account type" value={user.isBot ? 'Bot' : 'User'} />
          <DetailItem label="Created" value={formatDate(user.createdAt)} />
          <DetailItem label="Last login" value={formatDate(user.lastLoginAt)} />
        </CardContent>
      </Card>

      <AdminPasswordResetCard login={user.login} />

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

/**
 * Admin password reset. No current password is required — without this a
 * forgotten password has no recovery path short of env surgery.
 *
 * Only rendered under local auth; OAuth deployments have no password to reset.
 */
function AdminPasswordResetCard({ login }: { login: string }) {
  const { isPasswordMode } = useAuthMode()
  const [newPassword, setNewPassword] = useState('')

  const resetPassword = useMutation({
    mutationFn: (password: string) =>
      unwrapResponse(
        adminResetPasswordHandler(login, { newPassword: password, mustChangePassword: true }),
      ),
    onSuccess: () => {
      setNewPassword('')
      toast.success(`Password reset for ${login}. They must change it on next sign-in.`)
    },
    onError: (error) => {
      toast.error(`Failed to reset password: ${getErrorMessage(error)}`)
    },
  })

  if (!isPasswordMode) {
    return null
  }

  const tooShort = newPassword.length > 0 && newPassword.length < MIN_PASSWORD_LENGTH

  return (
    <Card>
      <CardHeader>
        <CardTitle>Reset password</CardTitle>
        <CardDescription>
          Set a new password for this user. Every active session is revoked, and they are prompted
          to change it on next sign-in.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="space-y-2">
          <Label htmlFor="admin-new-password">New password</Label>
          <Input
            id="admin-new-password"
            type="password"
            autoComplete="new-password"
            value={newPassword}
            onChange={(e) => setNewPassword(e.target.value)}
            placeholder={`At least ${MIN_PASSWORD_LENGTH} characters`}
          />
          {tooShort && (
            <p className="text-sm text-destructive">
              Password must be at least {MIN_PASSWORD_LENGTH} characters.
            </p>
          )}
        </div>
        <div className="flex justify-end">
          <Button
            onClick={() => resetPassword.mutate(newPassword)}
            disabled={newPassword.length < MIN_PASSWORD_LENGTH || resetPassword.isPending}
          >
            {resetPassword.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Reset Password
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
