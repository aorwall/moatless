import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminTeamDetail } from '@/features/admin/components/admin-team-detail'

export const Route = createLazyFileRoute('/admin/teams/$teamId/')({
  component: AdminTeamDetailPage,
})

function AdminTeamDetailPage() {
  const { teamId } = Route.useParams()
  return <AdminTeamDetail teamId={teamId} />
}
