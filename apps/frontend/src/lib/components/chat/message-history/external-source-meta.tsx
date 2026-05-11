import { GitPullRequest, Hash, Mail, Radio, Send, Zap } from 'lucide-react'

export function getAdapterIcon(adapterKind: string) {
  switch (adapterKind) {
    case 'slack':
      return <Hash className="h-3.5 w-3.5" />
    case 'webhook':
      return <Zap className="h-3.5 w-3.5" />
    case 'github':
      return <GitPullRequest className="h-3.5 w-3.5" />
    case 'telegram':
      return <Send className="h-3.5 w-3.5" />
    case 'email':
      return <Mail className="h-3.5 w-3.5" />
    default:
      return <Radio className="h-3.5 w-3.5" />
  }
}

export function getAdapterLabel(adapterKind: string): string {
  switch (adapterKind) {
    case 'slack':
      return 'Slack'
    case 'webhook':
      return 'Webhook'
    case 'github':
      return 'GitHub'
    case 'telegram':
      return 'Telegram'
    case 'email':
      return 'Email'
    default:
      return 'Integration'
  }
}
