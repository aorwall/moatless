import { createFileRoute } from '@tanstack/react-router'

import { RepositoryList, useRepositoriesList } from '@/features/repositories'

export const Route = createFileRoute('/repositories/')({
  component: RepositoriesIndex,
})

function RepositoriesIndex() {
  const { data: repositories, isLoading } = useRepositoriesList()

  return (
    <div className="container mx-auto py-6">
      <RepositoryList repositories={repositories} isLoading={isLoading} />
    </div>
  )
}
