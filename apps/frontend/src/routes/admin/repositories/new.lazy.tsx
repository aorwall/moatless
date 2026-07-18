import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { RepositoryConfigCreateSection } from '@/features/repositories/components/sections/repository-config-create-section'

export const Route = createLazyFileRoute('/admin/repositories/new')({
  component: NewRepositoryPage,
})

function NewRepositoryPage() {
  const navigate = useNavigate()

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">New repository</h1>
        <p className="text-sm text-muted-foreground">
          Connect a Git repository to use as a starting point for tasks.
        </p>
      </div>
      <RepositoryConfigCreateSection
        onCancel={() => navigate({ to: '/admin/repositories' })}
        onSuccess={(repositoryId) =>
          navigate({
            to: '/admin/repositories/$repositoryId',
            params: { repositoryId },
          })
        }
      />
    </div>
  )
}
