import { Link } from '@tanstack/react-router'
import { Loader2, Search, Users } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { AdminUserListItem } from '@/features/admin/hooks/use-admin-users-teams'
import { useAdminUsersList } from '@/features/admin/hooks/use-admin-users-teams'
import { Badge } from '@/lib/components/ui/badge'
import { Input } from '@/lib/components/ui/input'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { getErrorMessage } from '@/lib/utils/error'

function initials(user: AdminUserListItem): string {
  const source = user.name?.trim() || user.login
  return source.slice(0, 2).toUpperCase()
}

export function AdminUsersSidebar() {
  const { data, isLoading, error } = useAdminUsersList()
  const [search, setSearch] = useState('')

  const users = useMemo(() => {
    const all = data?.users ?? []
    const term = search.trim().toLowerCase()
    const filtered = term
      ? all.filter((user) =>
          [user.login, user.name, user.email]
            .filter((value): value is string => Boolean(value))
            .some((value) => value.toLowerCase().includes(term)),
        )
      : all
    return [...filtered].sort((a, b) => a.login.localeCompare(b.login))
  }, [data?.users, search])

  return (
    <aside className="flex h-full w-72 shrink-0 flex-col border-r bg-sidebar">
      <div className="space-y-3 border-b p-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 font-semibold">
            <Users className="h-4 w-4" />
            <span>Users</span>
          </div>
          {data ? <Badge variant="secondary">{data.users.length}</Badge> : null}
        </div>
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search users..."
            className="pl-8"
          />
        </div>
      </div>

      <ScrollArea className="flex-1">
        {isLoading ? (
          <div className="flex h-32 items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : error ? (
          <p className="p-4 text-sm text-destructive">{getErrorMessage(error)}</p>
        ) : users.length === 0 ? (
          <p className="p-4 text-sm text-muted-foreground">No users found</p>
        ) : (
          <nav className="p-2">
            {users.map((user) => (
              <Link
                key={user.id}
                to="/admin/users/$login"
                params={{ login: user.login }}
                activeProps={{ 'data-active': 'true' }}
                className="flex items-center gap-3 rounded-md px-2 py-2 text-sm hover:bg-sidebar-accent data-[active=true]:bg-sidebar-accent data-[active=true]:font-medium"
              >
                <span className="flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted text-xs font-medium text-muted-foreground">
                  {user.avatarUrl ? (
                    <img src={user.avatarUrl} alt="" className="h-full w-full object-cover" />
                  ) : (
                    initials(user)
                  )}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate font-medium">{user.name || user.login}</span>
                    {user.role === 'admin' ? (
                      <Badge variant="outline" className="h-4 px-1 text-[10px]">
                        admin
                      </Badge>
                    ) : null}
                  </span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {user.email || user.login}
                  </span>
                </span>
              </Link>
            ))}
          </nav>
        )}
      </ScrollArea>
    </aside>
  )
}
