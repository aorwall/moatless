import { createLazyFileRoute } from '@tanstack/react-router'
import { Package } from 'lucide-react'
import { DetailEmptyState } from '@/lib/components/ui/detail-empty-state'

export const Route = createLazyFileRoute('/admin/plugins/')({
  component: PluginsIndexPage,
})

function PluginsIndexPage() {
  return (
    <DetailEmptyState
      icon={Package}
      title="Select a plugin"
      description="Choose a plugin from the list to view its details."
    />
  )
}
