import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { WorkspaceConfigEditSection } from '@/features/workspaces/components/workspace-config-edit-section'

export const Route = createLazyFileRoute('/admin/workspaces/$workspaceId/')({
  component: AdminWorkspaceDetailPage,
})

function AdminWorkspaceDetailPage() {
  const { workspaceId } = Route.useParams()
  const navigate = useNavigate()

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <WorkspaceConfigEditSection
        key={workspaceId}
        workspaceId={workspaceId}
        onDeleted={() => navigate({ to: '/admin/workspaces' })}
      />
    </div>
  )
}
