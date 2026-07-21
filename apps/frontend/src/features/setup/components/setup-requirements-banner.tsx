import { SetupSeverity } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { AlertTriangle, X } from 'lucide-react'

import { useSettingsDialog } from '@/features/settings/contexts/settings-dialog-provider'
import { Button } from '@/lib/components/ui/button'
import { useSessionStorage } from '@/lib/hooks/use-session-storage'
import { cn } from '@/lib/utils'
import type { SetupItem } from '../hooks/use-setup-status'
import { useSetupStatus } from '../hooks/use-setup-status'

const DISMISSED_KEY = 'moatless.setup.dismissed'

/**
 * Persistent notice of what the environment still needs. Rendered above the
 * main region so it appears on every authenticated route, not just the home
 * page.
 */
export function SetupRequirementsBanner() {
  const { unmet } = useSetupStatus()
  const [dismissed, setDismissed] = useSessionStorage<string[]>(DISMISSED_KEY, [])

  // `required` items are never dismissible — their whole purpose is to stay
  // put until fixed, and each carries a one-click fix.
  const visible = unmet
    .filter((item) => item.severity === SetupSeverity.required || !dismissed.includes(item.id))
    .sort((a, b) => severityRank(a) - severityRank(b))

  if (visible.length === 0) {
    return null
  }

  const dismiss = (id: string) => {
    // Prune ids that are no longer reported, so a requirement that comes back
    // later isn't permanently muted by a stale entry.
    const live = new Set(unmet.map((item) => item.id))
    setDismissed([...dismissed.filter((entry) => live.has(entry)), id])
  }

  return (
    <div className="shrink-0" role="status" aria-live="polite">
      {visible.map((item) => (
        <SetupRequirementRow key={item.id} item={item} onDismiss={() => dismiss(item.id)} />
      ))}
    </div>
  )
}

function severityRank(item: SetupItem) {
  return item.severity === SetupSeverity.required ? 0 : 1
}

function SetupRequirementRow({ item, onDismiss }: { item: SetupItem; onDismiss: () => void }) {
  const { openSettings } = useSettingsDialog()
  const isRequired = item.severity === SetupSeverity.required

  return (
    <div
      className={cn(
        'flex items-center gap-3 border-b px-6 py-2 text-sm',
        isRequired
          ? 'border-amber-500/60 bg-amber-500/10 text-amber-700 dark:text-amber-400'
          : 'bg-muted/50 text-muted-foreground',
      )}
    >
      {isRequired && <AlertTriangle className="h-4 w-4 shrink-0" />}
      <div className="min-w-0 flex-1">
        <span className="font-medium">{item.copy.title}</span>
        <span className="ml-2 opacity-90">{item.copy.description}</span>
      </div>

      {item.action?.kind === 'route' && (
        <Button asChild variant="outline" size="sm" className="h-7 shrink-0">
          <Link to={item.action.to}>{item.action.label}</Link>
        </Button>
      )}
      {item.action?.kind === 'settings' && (
        <Button
          variant="outline"
          size="sm"
          className="h-7 shrink-0"
          onClick={() => {
            if (item.action?.kind === 'settings') {
              openSettings(item.action.section)
            }
          }}
        >
          {item.action.label}
        </Button>
      )}

      {!isRequired && (
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={onDismiss}
          aria-label={`Dismiss: ${item.copy.title}`}
        >
          <X className="h-4 w-4" />
        </Button>
      )}
    </div>
  )
}
