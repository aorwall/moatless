import type { ChannelInfo } from '@moatless/types'
import { Copy, Mail } from 'lucide-react'
import { toast } from 'sonner'
import { ConfigurationGroup, ConfigurationItem } from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'

interface ChannelsGroupProps {
  channels: ChannelInfo | undefined
}

export function ChannelsGroup({ channels }: ChannelsGroupProps) {
  // Only render if there are any channel integrations available
  if (!channels?.email) {
    return null
  }

  const handleCopyEmail = async () => {
    if (!channels.email) {
      return
    }

    try {
      await navigator.clipboard.writeText(channels.email.address)
      toast.success('Email address copied to clipboard')
    } catch {
      toast.error('Failed to copy email address')
    }
  }

  return (
    <ConfigurationGroup title="Channels">
      {channels.email && (
        <ConfigurationItem
          label="Email Address"
          description="Send emails to this address to create tasks for this repository"
        >
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Mail className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={channels.email.address}
                readOnly
                className="pl-10 font-mono text-sm bg-muted"
              />
            </div>
            <Button variant="outline" size="icon" onClick={handleCopyEmail} title="Copy email">
              <Copy className="h-4 w-4" />
            </Button>
          </div>
        </ConfigurationItem>
      )}
    </ConfigurationGroup>
  )
}
