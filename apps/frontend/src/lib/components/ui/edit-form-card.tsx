import { Loader2, Save } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'

interface EditFormCardProps {
  title: string
  description?: string
  isDirty: boolean
  isPending: boolean
  onSave: () => void
  onReset: () => void
  /** Optional label for the primary button. Defaults to "Save changes". */
  saveLabel?: string
  /** The form fields. */
  children: ReactNode
}

/**
 * A card wrapping editable fields with dirty-gated Save / Reset actions.
 * Pair with `useDirtyForm` for the dirty state.
 */
export function EditFormCard({
  title,
  description,
  isDirty,
  isPending,
  onSave,
  onReset,
  saveLabel = 'Save changes',
  children,
}: EditFormCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        {description ? <CardDescription>{description}</CardDescription> : null}
      </CardHeader>
      <CardContent className="space-y-4">
        {children}
        <div className="flex items-center gap-2 pt-2">
          <Button onClick={onSave} disabled={!isDirty || isPending}>
            {isPending ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Save className="mr-2 h-4 w-4" />
            )}
            {saveLabel}
          </Button>
          <Button variant="ghost" onClick={onReset} disabled={!isDirty || isPending}>
            Reset
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
