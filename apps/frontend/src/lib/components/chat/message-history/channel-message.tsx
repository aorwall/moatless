import type { UiMessage } from '@moatless/api-client'
import { Clock, GitPullRequest, Hash, Mail, Radio, Send, Zap } from 'lucide-react'

interface ChannelMessageProps {
  message: UiMessage
}

function getProviderIcon(provider: string) {
  switch (provider) {
    case 'slack':
      return <Hash className="h-3.5 w-3.5" />
    case 'cron':
      return <Clock className="h-3.5 w-3.5" />
    case 'webhook':
      return <Zap className="h-3.5 w-3.5" />
    case 'github_pr':
      return <GitPullRequest className="h-3.5 w-3.5" />
    case 'telegram':
      return <Send className="h-3.5 w-3.5" />
    case 'email':
      return <Mail className="h-3.5 w-3.5" />
    default:
      return <Radio className="h-3.5 w-3.5" />
  }
}

function getProviderLabel(provider: string): string {
  switch (provider) {
    case 'slack':
      return 'Slack'
    case 'cron':
      return 'Scheduled'
    case 'webhook':
      return 'Webhook'
    case 'github_pr':
      return 'GitHub PR'
    case 'telegram':
      return 'Telegram'
    case 'email':
      return 'Email'
    default:
      return 'Channel'
  }
}

function getTextContent(message: UiMessage): string {
  return message.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text ?? '')
    .join('\n')
}

function formatGithubExternalId(kind: string, rest: string[], raw: string): string {
  const single = rest.length === 1 ? rest[0] : null
  switch (kind) {
    case 'ic':
      return single ? `Comment #${single}` : raw
    case 'rc':
    case 'rv':
      return single ? `Review #${single}` : raw
    case 'cr':
      return single ? `Check #${single}` : raw
    case 'cs':
      return single ? `Suite #${single}` : raw
    case 'pr':
      if (rest.length === 2) {
        return `PR #${rest[1]} ${rest[0]}`
      }
      if (rest.length === 3) {
        return `PR #${rest[1]} ${rest[0]} ${rest[2]}`
      }
      return raw
    default:
      return raw
  }
}

export function formatExternalId(raw: string): string {
  const parts = raw.split(':')
  if (parts.length < 2) {
    return raw
  }

  const [namespace, kind, ...rest] = parts

  if (namespace === 'github') {
    return formatGithubExternalId(kind, rest, raw)
  }

  if (namespace === 'slack' && kind === 'ts' && rest.length === 1) {
    return `ts ${rest[0]}`
  }

  if (namespace === 'tg' && kind === 'm' && rest.length === 1) {
    return `msg #${rest[0]}`
  }

  return raw
}

export function ChannelMessageComponent({ message }: ChannelMessageProps) {
  const textContent = getTextContent(message)
  const channel = message.channel
  const provider = channel?.provider ?? 'channel'
  const sourceName = channel?.sourceName
  const externalId = channel?.externalId

  return (
    <div className="w-full rounded-lg border-l-4 border-blue-500/50 bg-blue-500/5 dark:bg-blue-500/10 overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground border-b border-blue-500/10">
        <span className="text-blue-500/70">{getProviderIcon(provider)}</span>
        <span className="font-medium">via {getProviderLabel(provider)}</span>
        {sourceName && <span className="text-muted-foreground/70">{sourceName}</span>}
        {externalId && (
          <span
            className="ml-auto font-mono text-[10px] text-muted-foreground/60"
            title={externalId}
          >
            {formatExternalId(externalId)}
          </span>
        )}
      </div>
      <pre className="px-3 py-2 text-sm font-mono whitespace-pre-wrap break-words overflow-x-auto text-foreground/90">
        {textContent}
      </pre>
    </div>
  )
}
