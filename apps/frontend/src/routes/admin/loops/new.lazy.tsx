import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { LoopForm } from '@/features/loops/components/loop-form'

export const Route = createLazyFileRoute('/admin/loops/new')({
  component: NewLoopPage,
})

function NewLoopPage() {
  const navigate = useNavigate()
  const goBack = () => navigate({ to: '/admin/loops' })

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">New Loop</h1>
        <p className="text-sm text-muted-foreground">
          Create a subscription or schedule loop with its embedded rule.
        </p>
      </div>
      <LoopForm loop={null} onSuccess={goBack} onCancel={goBack} />
    </div>
  )
}
