import { createLazyFileRoute } from '@tanstack/react-router'
import { SandboxOverview } from '@/features/sandbox/components/sandbox-overview'

export const Route = createLazyFileRoute('/sandbox/')({
  component: SandboxOverviewPage,
})

function SandboxOverviewPage() {
  return (
    <div className="container mx-auto max-w-7xl px-6 py-6">
      <SandboxOverview />
    </div>
  )
}
