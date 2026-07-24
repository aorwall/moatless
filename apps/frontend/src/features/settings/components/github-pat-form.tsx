import { Loader2, LogOut } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'

interface GitHubPatFormProps {
  /** Drives Save-vs-Update wording and whether Remove is offered. */
  hasToken: boolean
  /** A mutation's `mutateAsync`. */
  onSave: (token: string) => Promise<unknown>
  isSaving: boolean
  onRemove?: (() => void) | undefined
  isRemoving?: boolean | undefined
  removeLabel?: string | undefined
  placeholder?: string | undefined
}

/**
 * Password input + save/remove controls for a user's GitHub PAT.
 */
export function GitHubPatForm({
  hasToken,
  onSave,
  isSaving,
  onRemove,
  isRemoving = false,
  removeLabel = 'Remove',
  placeholder,
}: GitHubPatFormProps) {
  const [pat, setPat] = useState('')

  const handleSave = async () => {
    const token = pat.trim()
    if (!token) {
      toast.error('Please enter a Personal Access Token')
      return
    }

    try {
      await onSave(token)
      // Only clear on success — a rejected save should leave the value in
      // place so the user can retry without re-pasting.
      setPat('')
    } catch {
      // The calling mutation surfaces the error; nothing to add here.
    }
  }

  const defaultPlaceholder = hasToken
    ? 'Enter a new token to replace the current one'
    : 'Enter a GitHub personal access token'

  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <Input
          type="password"
          placeholder={placeholder ?? defaultPlaceholder}
          value={pat}
          onChange={(e) => setPat(e.target.value)}
          className="flex-1 font-mono"
        />
        <Button onClick={handleSave} disabled={isSaving || !pat.trim()} variant="outline">
          {isSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : hasToken ? 'Update' : 'Save'}
        </Button>
      </div>

      {hasToken && onRemove && (
        <div className="flex justify-end">
          <Button onClick={onRemove} disabled={isRemoving} variant="outline" size="sm">
            {isRemoving ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <LogOut className="mr-2 h-4 w-4" />
            )}
            {removeLabel}
          </Button>
        </div>
      )}
    </div>
  )
}
