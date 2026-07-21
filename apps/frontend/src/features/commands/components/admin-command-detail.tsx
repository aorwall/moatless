import { useNavigate } from '@tanstack/react-router'
import { Loader2, Trash2 } from 'lucide-react'
import { createElement, useState } from 'react'
import { toast } from 'sonner'
import { CommandForm } from '@/features/commands/components/command-form'
import { useCommand, useDeleteCommand } from '@/features/commands/hooks/use-commands'
import { getLucideIcon } from '@/features/commands/utils/lucide-icon'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/lib/components/ui/alert-dialog'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { DetailHeader } from '@/lib/components/ui/detail-header'
import { DetailNotFound } from '@/lib/components/ui/detail-not-found'
import { getErrorMessage } from '@/lib/utils/error'

export function AdminCommandDetail({ name }: { name: string }) {
  const navigate = useNavigate()
  const { data: command, isLoading, error } = useCommand(name)
  const deleteCommand = useDeleteCommand()
  const [confirmOpen, setConfirmOpen] = useState(false)

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (error || !command) {
    return <DetailNotFound title="Command not found" description={`No command named “${name}”.`} />
  }

  const IconComponent = command.icon ? getLucideIcon(command.icon) : null

  const handleDelete = () => {
    deleteCommand.mutate(command.name, {
      onSuccess: async () => {
        toast.success('Command deleted successfully')
        setConfirmOpen(false)
        await navigate({ to: '/admin/commands' })
      },
      onError: (err) =>
        toast.error('Failed to delete command', { description: getErrorMessage(err) }),
    })
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <DetailHeader
        avatar={
          IconComponent ? createElement(IconComponent, { className: 'mt-1 h-6 w-6' }) : undefined
        }
        title={command.name}
        subtitle={command.description}
        badges={command.active === false ? <Badge variant="outline">inactive</Badge> : null}
        actions={
          <Button
            onClick={() => setConfirmOpen(true)}
            size="sm"
            variant="destructive"
            disabled={deleteCommand.isPending}
          >
            <Trash2 className="mr-2 h-4 w-4" />
            Delete
          </Button>
        }
      />

      <CommandForm key={command.name} command={command} />

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete command?</AlertDialogTitle>
            <AlertDialogDescription>
              This permanently deletes "{command.name}". This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteCommand.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(event) => {
                event.preventDefault()
                handleDelete()
              }}
              disabled={deleteCommand.isPending}
            >
              {deleteCommand.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
