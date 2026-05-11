import { AdapterKind } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import {
  useCreateIntegrationConnection,
  useIntegrationAdapters,
} from '@/features/integrations/hooks/use-integrations'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

type JsonObject = Record<string, unknown>

interface ConnectionFormDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

interface ConnectionFormState {
  adapter: string
  externalAccountId: string
  sourceName: string
  connectionKind: string
  webhookSecret: string
  transportConfigJson: string
}

const ADAPTER_LABELS: Record<string, string> = {
  slack: 'Slack',
  telegram: 'Telegram',
  github: 'GitHub',
  linear: 'Linear',
  webhook: 'Webhook',
}

const ADAPTER_VALUES = Object.values(AdapterKind)

function getAdapterLabel(adapter: string): string {
  return ADAPTER_LABELS[adapter] ?? adapter
}

function toAdapterKind(adapter: string): AdapterKind | null {
  return ADAPTER_VALUES.includes(adapter as AdapterKind) ? (adapter as AdapterKind) : null
}

function defaultConnectionKind(adapter: string): string {
  if (adapter === 'webhook') {
    return 'webhook'
  }
  if (adapter === 'github') {
    return 'org_webhook'
  }
  return 'bot_api'
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function parseJsonObject(value: string, label: string): JsonObject | null {
  const trimmed = value.trim()
  if (!trimmed) {
    return null
  }
  const parsed: unknown = JSON.parse(trimmed)
  if (parsed === null) {
    return null
  }
  if (!isJsonObject(parsed)) {
    throw new Error(`${label} must be a JSON object`)
  }
  return parsed
}

function makeInitialForm(defaultAdapter: string): ConnectionFormState {
  return {
    adapter: defaultAdapter,
    externalAccountId: '',
    sourceName: '',
    connectionKind: defaultConnectionKind(defaultAdapter),
    webhookSecret: '',
    transportConfigJson: '{}',
  }
}

export function ConnectionFormDialog({ open, onOpenChange }: ConnectionFormDialogProps) {
  const { data: adapters } = useIntegrationAdapters()
  const adapterItems = adapters ?? []
  const defaultAdapter = adapterItems[0] ?? AdapterKind.slack
  const [form, setForm] = useState<ConnectionFormState>(() => makeInitialForm(defaultAdapter))
  const createConnection = useCreateIntegrationConnection()
  const isPending = createConnection.isPending

  useEffect(() => {
    if (open) {
      setForm(makeInitialForm(defaultAdapter))
    }
  }, [open, defaultAdapter])

  const setAdapter = (adapter: string) => {
    setForm((prev) => ({
      ...prev,
      adapter,
      connectionKind: defaultConnectionKind(adapter),
    }))
  }

  const handleSubmit = async () => {
    const adapterKind = toAdapterKind(form.adapter)
    if (!adapterKind) {
      toast.error('Select an adapter')
      return
    }
    if (!form.externalAccountId.trim()) {
      toast.error('Enter an external account ID')
      return
    }

    try {
      const transportConfig = parseJsonObject(form.transportConfigJson, 'Transport Config JSON')
      await createConnection.mutateAsync({
        adapterKind,
        externalAccountId: form.externalAccountId.trim(),
        sourceName: form.sourceName.trim() || null,
        connectionKind: form.connectionKind.trim() || null,
        webhookSecret: form.webhookSecret.trim() || null,
        transportConfig,
      })
      toast.success('Connection created')
      onOpenChange(false)
    } catch (error) {
      toast.error('Failed to create connection', {
        description: getErrorMessage(error),
      })
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>New Connection</DialogTitle>
          <DialogDescription>
            Create a new adapter connection for triggers to use.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <div className="space-y-2">
            <Label>Adapter</Label>
            <Select value={form.adapter} onValueChange={setAdapter}>
              <SelectTrigger>
                <SelectValue placeholder="Select adapter" />
              </SelectTrigger>
              <SelectContent>
                {adapterItems.map((adapter) => (
                  <SelectItem key={adapter} value={adapter}>
                    {getAdapterLabel(adapter)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label>External Account ID</Label>
              <Input
                value={form.externalAccountId}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, externalAccountId: event.target.value }))
                }
                placeholder="workspace, org, or account ID"
              />
            </div>
            <div className="space-y-2">
              <Label>
                Name <span className="text-muted-foreground">(optional)</span>
              </Label>
              <Input
                value={form.sourceName}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, sourceName: event.target.value }))
                }
                placeholder="Display name"
              />
            </div>
            <div className="space-y-2">
              <Label>Connection Kind</Label>
              <Input
                value={form.connectionKind}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, connectionKind: event.target.value }))
                }
                placeholder="bot_api"
              />
            </div>
            <div className="space-y-2">
              <Label>
                Webhook Secret <span className="text-muted-foreground">(optional)</span>
              </Label>
              <Input
                type="password"
                value={form.webhookSecret}
                onChange={(event) =>
                  setForm((prev) => ({ ...prev, webhookSecret: event.target.value }))
                }
                placeholder="Secret"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label>
              Transport Config JSON <span className="text-muted-foreground">(optional)</span>
            </Label>
            <Textarea
              value={form.transportConfigJson}
              onChange={(event) =>
                setForm((prev) => ({ ...prev, transportConfigJson: event.target.value }))
              }
              rows={3}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={isPending || !form.externalAccountId.trim() || adapterItems.length === 0}
          >
            {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
