import { Check, Copy } from 'lucide-react'
import { useCallback, useState } from 'react'
import type { MessageAction } from '../message-action-bar'

/**
 * Build a Copy action for the action bar. Copies `text` to the clipboard via
 * `navigator.clipboard.writeText`.
 */
export function useCopyAction(text: string): MessageAction {
  const [copied, setCopied] = useState(false)

  const onClick = useCallback(() => {
    if (!text) {
      return
    }
    if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
      navigator.clipboard
        .writeText(text)
        .then(() => {
          setCopied(true)
          window.setTimeout(() => setCopied(false), 1500)
        })
        .catch(() => {
          // Best-effort: copying isn't critical; ignore failures.
        })
    }
  }, [text])

  return {
    key: 'copy',
    icon: copied ? Check : Copy,
    label: copied ? 'Copied' : 'Copy message',
    onClick,
  }
}
