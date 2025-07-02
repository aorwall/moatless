import { getErrorMessage } from '@moatless/shared/types/api-errors'
import type { CreateCommandRequest } from '@moatless/shared/types/command'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'

import { CommandForm, useCreateCommand } from '@/features/commands'

export const Route = createFileRoute('/commands/new')({
  component: NewCommand,
})

function NewCommand() {
  const navigate = useNavigate()
  const createCommand = useCreateCommand({
    onSuccess: (data) => {
      toast.success(`Command "${data.name}" created successfully`)
      navigate({ to: '/commands' })
    },
    onError: (error) => {
      toast.error(getErrorMessage(error))
    },
  })

  return (
    <div className="container mx-auto py-6 max-w-2xl">
      <CommandForm
        onSubmit={(data) => createCommand.mutate(data as CreateCommandRequest)}
        isSubmitting={createCommand.isPending}
      />
    </div>
  )
}
