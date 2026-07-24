import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { Loader2 } from 'lucide-react'
import { LoopForm } from '@/features/loops/components/loop-form'
import { useLoop } from '@/features/loops/hooks/use-loops'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'

export const Route = createLazyFileRoute('/admin/loops/$loop-id/edit')({
  component: EditLoopPage,
})

function EditLoopPage() {
  const params = Route.useParams()
  const loopId = params['loop-id']
  const navigate = useNavigate()
  const { data: loop, isLoading } = useLoop(loopId)

  const goBack = () =>
    navigate({
      to: '/admin/loops/$loop-id',
      params: { 'loop-id': loopId },
    })

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!loop) {
    return <DetailNotFound title="Loop not found" description="This loop may have been deleted." />
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">Edit Loop</h1>
        <p className="text-sm text-muted-foreground">Update loop source and rule fields.</p>
      </div>
      <LoopForm loop={loop} onSuccess={goBack} onCancel={goBack} />
    </div>
  )
}
