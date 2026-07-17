import { UserRole } from '@moatless/api-client'
import { createLazyFileRoute } from '@tanstack/react-router'
import { Loader2, Save, Users } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { AdminPage } from '@/features/admin'
import {
  useAdminTeamMembersByTeam,
  useAdminTeamsList,
  useAdminUsersList,
  useUpdateAdminUser,
} from '@/features/admin/hooks/use-admin-users-teams'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
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
import { getErrorMessage } from '@/lib/utils/error'

type UserRoleValue = (typeof UserRole)[keyof typeof UserRole]

interface AdminUserRow {
  id: string
  login: string
  email?: string | null
  name?: string | null
  avatarUrl?: string | null
  provider?: string
  role?: UserRoleValue
  lastLoginAt?: string
  isBot?: boolean
}

interface UserMembership {
  teamId: string
  teamName: string
  role: string
}

function formatDate(value: string | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

export const Route = createLazyFileRoute('/admin/users/')({
  component: AdminUsersPage,
})

function AdminUsersPage() {
  const usersQuery = useAdminUsersList()
  const teamsQuery = useAdminTeamsList()
  const teams = teamsQuery.data ?? []
  const memberQueries = useAdminTeamMembersByTeam(teams)

  const membershipsByUser = useMemo(() => {
    const memberships = new Map<string, UserMembership[]>()
    for (const [index, team] of teams.entries()) {
      const members = memberQueries[index]?.data ?? []
      for (const member of members) {
        const userMemberships = memberships.get(member.userId) ?? []
        userMemberships.push({ teamId: team.id, teamName: team.name, role: member.role })
        memberships.set(member.userId, userMemberships)
      }
    }
    return memberships
  }, [teams, memberQueries])

  const users = usersQuery.data?.users ?? []
  const isLoading =
    usersQuery.isLoading || teamsQuery.isLoading || memberQueries.some((query) => query.isLoading)
  const error =
    usersQuery.error ?? teamsQuery.error ?? memberQueries.find((query) => query.error)?.error

  return (
    <AdminPage
      title="Users"
      description="Review registered users, update profile fields, assign global roles, and inspect team membership."
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
      ) : users.length === 0 ? (
        <div className="rounded-md border py-12 text-center text-muted-foreground">
          <Users className="mx-auto mb-3 h-10 w-10 opacity-50" />
          <p>No users found</p>
        </div>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>User</TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Email</TableHead>
              <TableHead>Global Role</TableHead>
              <TableHead>Teams</TableHead>
              <TableHead>Last Login</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {users.map((user) => (
              <UserAdminRow
                key={user.id}
                user={user}
                memberships={membershipsByUser.get(user.id) ?? []}
              />
            ))}
          </TableBody>
        </Table>
      )}
    </AdminPage>
  )
}

function UserAdminRow({
  user,
  memberships,
}: {
  user: AdminUserRow
  memberships: UserMembership[]
}) {
  const updateUser = useUpdateAdminUser()
  const [name, setName] = useState(user.name ?? '')
  const [email, setEmail] = useState(user.email ?? '')
  const [role, setRole] = useState<UserRoleValue>(user.role ?? UserRole.user)

  const saveDisabled =
    updateUser.isPending ||
    (name === (user.name ?? '') &&
      email === (user.email ?? '') &&
      role === (user.role ?? UserRole.user))

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
        onError: (error) =>
          toast.error('Failed to update user', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <TableRow className="align-top">
      <TableCell>
        <div className="font-medium">{user.login}</div>
        <div className="text-xs text-muted-foreground">
          {user.provider ?? '-'} {user.isBot ? 'bot' : 'user'}
        </div>
      </TableCell>
      <TableCell>
        <Input value={name} onChange={(event) => setName(event.target.value)} />
      </TableCell>
      <TableCell>
        <Input value={email} onChange={(event) => setEmail(event.target.value)} />
      </TableCell>
      <TableCell>
        <Select value={role} onValueChange={(value) => setRole(value as UserRoleValue)}>
          <SelectTrigger className="w-28">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={UserRole.user}>User</SelectItem>
            <SelectItem value={UserRole.admin}>Admin</SelectItem>
          </SelectContent>
        </Select>
      </TableCell>
      <TableCell>
        {memberships.length === 0 ? (
          <span className="text-sm text-muted-foreground">-</span>
        ) : (
          <div className="flex max-w-sm flex-wrap gap-1">
            {memberships.map((membership) => (
              <Badge key={membership.teamId} variant="outline">
                {membership.teamName}: {membership.role}
              </Badge>
            ))}
          </div>
        )}
      </TableCell>
      <TableCell className="text-sm text-muted-foreground">
        {formatDate(user.lastLoginAt)}
      </TableCell>
      <TableCell className="text-right">
        <Button size="sm" onClick={handleSave} disabled={saveDisabled}>
          {updateUser.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Save className="h-4 w-4" />
          )}
        </Button>
      </TableCell>
    </TableRow>
  )
}
