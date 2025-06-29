import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'

import { CommandForm, useCommand, useUpdateCommand } from '@/features/commands'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'

export const Route = createFileRoute('/commands/$commandName')({
  component: EditCommand,
})

function EditCommand() {
  const navigate = useNavigate()
  const { commandName } = Route.useParams()
  const { data: command, isLoading, error } = useCommand(commandName)

  const updateCommand = useUpdateCommand({
    onSuccess: (data) => {
      toast.success(`Command "${data.name}" updated successfully`)
      if (data.name !== commandName) {
        // If name changed, navigate to new URL
        navigate({ to: '/commands/$commandName', params: { commandName: data.name } })
      }
    },
    onError: (error) => {
      toast.error(error.message || 'Failed to update command')
    },
  })

  if (error) {
    return (
      <div className="container mx-auto py-6 max-w-2xl">
        <ErrorAlert error={error} />
      </div>
    )
  }

  if (isLoading) {
    return (
      <div className="container mx-auto py-6 max-w-2xl">
        <div className="space-y-4">
          <Skeleton className="h-10 w-1/3" />
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-32 w-full" />
        </div>
      </div>
    )
  }

  if (!command) {
    return (
      <div className="container mx-auto py-6 max-w-2xl">
        <ErrorAlert error={new Error('Command not found')} />
      </div>
    )
  }

  return (
    <div className="container mx-auto py-6 max-w-2xl">
      <CommandForm
        command={command}
        onSubmit={(data) => updateCommand.mutate({ commandName, request: data })}
        isSubmitting={updateCommand.isPending}
      />
    </div>
  )
}
