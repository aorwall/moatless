import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminUserProfile } from '@/features/admin'

export const Route = createLazyFileRoute('/admin/users/$login')({
  component: AdminUserProfilePage,
})

function AdminUserProfilePage() {
  const { login } = Route.useParams()
  return <AdminUserProfile login={login} />
}
