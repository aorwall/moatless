import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { RepositoryConfigEditSection } from '@/features/repositories/components/sections/repository-config-edit-section'

export const Route = createLazyFileRoute('/admin/repositories/$repositoryId/')({
  component: AdminRepositoryDetailPage,
})

function AdminRepositoryDetailPage() {
  const { repositoryId } = Route.useParams()
  const navigate = useNavigate()

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <RepositoryConfigEditSection
        key={repositoryId}
        repositoryId={repositoryId}
        onDeleted={() => navigate({ to: '/admin/repositories' })}
      />
    </div>
  )
}
