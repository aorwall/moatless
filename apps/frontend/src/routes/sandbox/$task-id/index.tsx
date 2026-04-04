import { createFileRoute } from '@tanstack/react-router'
import { SandboxDiagnostics } from '@/features/sandbox/components/sandbox-diagnostics'

export const Route = createFileRoute('/sandbox/$task-id/')({
  component: SandboxDiagnosticsPage,
})

function SandboxDiagnosticsPage() {
  const { 'task-id': taskId } = Route.useParams()
  return (
    <div className="container mx-auto max-w-7xl px-6 py-6">
      <SandboxDiagnostics taskId={taskId} />
    </div>
  )
}
