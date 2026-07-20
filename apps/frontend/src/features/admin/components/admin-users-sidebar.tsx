import { Users } from 'lucide-react'
import { useState } from 'react'
import type { AdminUserListItem } from '@/features/admin/hooks/use-admin-users-teams'
import { useAdminUsersList } from '@/features/admin/hooks/use-admin-users-teams'
import { Badge } from '@/lib/components/ui/badge'
import { ListAvatar } from '@/lib/components/ui/list-avatar'
import { ListDetailSidebar } from '@/lib/components/ui/list-detail-sidebar'
import { useListFilter } from '@/lib/hooks/use-list-filter'

const getSearchText = (user: AdminUserListItem) => [user.login, user.name, user.email]
const sortByLogin = (a: AdminUserListItem, b: AdminUserListItem) => a.login.localeCompare(b.login)

export function AdminUsersSidebar() {
  const { data, isLoading, error } = useAdminUsersList()
  const [search, setSearch] = useState('')
  const users = useListFilter(data?.users, search, getSearchText, sortByLogin)

  return (
    <ListDetailSidebar
      icon={Users}
      title="Users"
      count={data?.users.length}
      search={search}
      onSearchChange={setSearch}
      searchPlaceholder="Search users..."
      isLoading={isLoading}
      error={error}
      isEmpty={users.length === 0}
      emptyMessage="No users found"
    >
      {users.map((user) => (
        <ListDetailSidebar.Item
          key={user.id}
          to="/admin/users/$login"
          params={{ login: user.login }}
          leading={<ListAvatar src={user.avatarUrl} label={user.name?.trim() || user.login} />}
          primary={user.name || user.login}
          secondary={user.email || user.login}
          trailing={
            user.role === 'admin' ? (
              <Badge variant="outline" className="h-4 px-1 text-[10px]">
                admin
              </Badge>
            ) : null
          }
        />
      ))}
    </ListDetailSidebar>
  )
}
