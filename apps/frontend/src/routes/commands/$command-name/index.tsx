import { getErrorMessage } from '@moatless/types'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'

import { CommandForm, useCommand, useUpdateCommand } from '@/features/commands'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'

export const Route = createFileRoute('/commands/$command-name/')({
  component: EditCommand,
})

function EditCommand() {
  const navigate = useNavigate()
  const { 'command-name': commandName } = Route.useParams()
  const { data: command, isLoading, error } = useCommand(commandName)

  const updateCommand = useUpdateCommand({
    onSuccess: (data) => {
      toast.success(`Command "${data.name}" updated successfully`)
      if (data.name !== commandName) {
        // If name changed, navigate to new URL
        navigate({
          to: '/commands/$command-name',
          params: { 'command-name': data.name },
        })
      }
    },
    onError: (error) => {
      toast.error(getErrorMessage(error))
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
