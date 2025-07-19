import { createFileRoute } from '@tanstack/react-router'
import { Settings } from 'lucide-react'
import { useState } from 'react'
import {
  RepositoriesConfigurationDialog,
  RepositoryList,
  useRepositoriesList,
} from '@/features/repositories'
import { Button } from '@/lib/components/ui/button'

export const Route = createFileRoute('/repositories/')({
  component: RepositoriesIndex,
})

function RepositoriesIndex() {
  const { data: repositories, isLoading } = useRepositoriesList()
  const [configOpen, setConfigOpen] = useState(false)

  return (
    <div className="container mx-auto py-6">
      <div className="space-y-4">
        {/* Add Configure button to the header */}
        {repositories && repositories.length > 0 && (
          <div className="flex justify-end">
            <Button onClick={() => setConfigOpen(true)}>
              <Settings className="h-4 w-4 mr-2" />
              Configure Repositories
            </Button>
          </div>
        )}

        <RepositoryList repositories={repositories} isLoading={isLoading} />

        <RepositoriesConfigurationDialog open={configOpen} onOpenChange={setConfigOpen} />
      </div>
    </div>
  )
}
