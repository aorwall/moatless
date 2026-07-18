import { createLazyFileRoute } from '@tanstack/react-router'
import { AdminCommandDetail } from '@/features/commands/components/admin-command-detail'

export const Route = createLazyFileRoute('/admin/commands/$name/')({
  component: CommandDetailPage,
})

function CommandDetailPage() {
  const { name } = Route.useParams()
  return <AdminCommandDetail name={name} />
}
