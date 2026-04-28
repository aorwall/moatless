import type { SecretMetadataResponse } from '@moatless/api-client'
import { Loader2, Pencil, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useDeleteSecret } from '@/features/secrets/hooks/use-secrets'
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { getErrorMessage } from '@/lib/utils/error'

interface SecretsListProps {
  secrets: SecretMetadataResponse[]
  isLoading: boolean
  error: Error | null
  onEdit: (secret: SecretMetadataResponse) => void
}

function formatUpdated(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) {
    return iso
  }
  return date.toLocaleString()
}

export function SecretsList({ secrets, isLoading, error, onEdit }: SecretsListProps) {
  const deleteSecret = useDeleteSecret()
  const [pendingDelete, setPendingDelete] = useState<SecretMetadataResponse | null>(null)

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12 text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        <span className="ml-2">Loading secrets…</span>
      </div>
    )
  }

  if (error) {
    return (
      <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
        {getErrorMessage(error)}
      </div>
    )
  }

  if (secrets.length === 0) {
    return (
      <div className="rounded-md border border-dashed py-12 text-center text-sm text-muted-foreground">
        No secrets at this scope yet.
      </div>
    )
  }

  const confirmDelete = () => {
    if (!pendingDelete) {
      return
    }
    const id = pendingDelete.id
    const key = pendingDelete.key
    deleteSecret.mutate(id, {
      onSuccess: () => {
        toast.success(`Deleted ${key}`)
        setPendingDelete(null)
      },
      onError: (err) => {
        toast.error('Failed to delete secret', { description: getErrorMessage(err) })
      },
    })
  }

  return (
    <>
      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Key</TableHead>
              <TableHead>Kind</TableHead>
              <TableHead>Updated</TableHead>
              <TableHead className="w-[100px] text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {secrets.map((secret) => (
              <TableRow key={secret.id}>
                <TableCell className="font-mono text-sm">{secret.key}</TableCell>
                <TableCell>
                  <Badge variant="secondary">{secret.kind}</Badge>
                </TableCell>
                <TableCell className="text-sm text-muted-foreground">
                  {formatUpdated(secret.updatedAt)}
                </TableCell>
                <TableCell className="text-right">
                  <Button variant="ghost" size="icon" onClick={() => onEdit(secret)} title="Edit">
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setPendingDelete(secret)}
                    title="Delete"
                  >
                    <Trash2 className="h-4 w-4 text-destructive" />
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      <AlertDialog
        open={!!pendingDelete}
        onOpenChange={(open) => {
          if (!open) {
            setPendingDelete(null)
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {pendingDelete?.key}?</AlertDialogTitle>
            <AlertDialogDescription>
              This permanently removes the secret. Running tasks that already resolved it keep the
              value, but new tasks will not receive it.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteSecret.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmDelete}
              disabled={deleteSecret.isPending}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteSecret.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
