import { createLazyFileRoute } from '@tanstack/react-router'
import { UserRound } from 'lucide-react'

export const Route = createLazyFileRoute('/admin/users/')({
  component: AdminUsersIndexPage,
})

function AdminUsersIndexPage() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
      <UserRound className="h-10 w-10 opacity-50" />
      <p className="font-medium">Select a user</p>
      <p className="text-sm">Choose a user from the list to view and edit their profile.</p>
    </div>
  )
}
