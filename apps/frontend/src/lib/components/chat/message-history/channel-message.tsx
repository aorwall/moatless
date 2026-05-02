import type { UiMessage } from '@moatless/api-client'
import { getProviderIcon, getProviderLabel } from './channel-meta'

interface ChannelMessageProps {
  message: UiMessage
}

function getTextContent(message: UiMessage): string {
  return message.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text ?? '')
    .join('\n')
}

export function ChannelMessageComponent({ message }: ChannelMessageProps) {
  const textContent = getTextContent(message)
  const channel = message.channel
  const provider = channel?.provider ?? 'channel'
  const sourceName = channel?.sourceName
  const externalId = channel?.externalId
  const externalLabel = channel?.externalLabel ?? externalId

  return (
    <div className="w-full rounded-lg border-l-4 border-blue-500/50 bg-blue-500/5 dark:bg-blue-500/10 overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground border-b border-blue-500/10">
        <span className="text-blue-500/70">{getProviderIcon(provider)}</span>
        <span className="font-medium">via {getProviderLabel(provider)}</span>
        {sourceName && <span className="text-muted-foreground/70">{sourceName}</span>}
        {externalLabel && (
          <span
            className="ml-auto font-mono text-[10px] text-muted-foreground/60"
            title={externalId ?? externalLabel}
          >
            {externalLabel}
          </span>
        )}
      </div>
      <pre className="px-3 py-2 text-sm font-mono whitespace-pre-wrap break-words overflow-x-auto text-foreground/90">
        {textContent}
      </pre>
    </div>
  )
}
