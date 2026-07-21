import type { UiMessage } from '@moatless/api-client'
import { AlertTriangle } from 'lucide-react'

interface ErrorMessageProps {
  message: UiMessage
}

function getErrorText(message: UiMessage): string {
  const text = message.content
    .flatMap((c) => (c.type === 'text' ? [c.text ?? ''] : []))
    .join('\n')
    .trim()
  return text || 'An error occurred.'
}

export function ErrorMessageComponent({ message }: ErrorMessageProps) {
  const text = getErrorText(message)
  const willNotResume = message.result?.willNotResume === true

  return (
    <div
      role="alert"
      className="border border-red-500/40 rounded-lg bg-red-500/5 p-3 flex items-start gap-2"
    >
      <AlertTriangle className="h-4 w-4 flex-shrink-0 text-red-500 mt-0.5" />
      <div className="flex-1 min-w-0">
        <div className="text-xs font-medium text-red-600 dark:text-red-400 mb-1">Error</div>
        <div className="text-sm text-red-600 dark:text-red-400 whitespace-pre-wrap break-words">
          {text}
        </div>
        {willNotResume && (
          <div className="mt-2 text-xs text-red-500/80 dark:text-red-400/80">
            Session will not resume.
          </div>
        )}
      </div>
    </div>
  )
}
