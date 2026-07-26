import type {
  ActivationResponse,
  EffectivePluginResponse,
  PluginSkillResponse,
} from '@moatless/api-client'
import { ActivationReach } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  useClearActivation,
  useEffectivePlugins,
  usePluginActivations,
  usePluginSkills,
  useSetActivation,
} from '@/features/plugins/hooks/use-plugins'
import { Badge } from '@/lib/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { getErrorMessage } from '@/lib/utils/error'

/** What one select can say. `unset` means "no record" — not the same as off. */
const SETTINGS = ['on', 'off', 'unset'] as const
type Setting = (typeof SETTINGS)[number]

/** One row: the plugin as a whole, or one skill inside it. */
type Row = {
  /** `undefined` means the whole plugin. */
  skillName: string | undefined
  label: string
  description: string | undefined
}

/**
 * The controls block, sized once so the header labels line up with the rows
 * beneath them without a table. Each control is fixed-width and never shrinks —
 * a long skill description wraps in the space left over rather than pushing the
 * controls off the right-hand edge.
 */
const CONTROLS = 'flex shrink-0 items-center gap-3'
const CONTROL_WIDTH = 'w-32'
const DELIVERED_WIDTH = 'w-20'

function settingOf(
  records: ActivationResponse[],
  reach: ActivationReach,
  skillName: string | undefined,
): Setting {
  // The wire omits `skillName` for a whole-plugin record; tolerate an explicit
  // null for the same thing.
  const record = records.find((r) => r.reach === reach && (r.skillName ?? undefined) === skillName)
  if (record === undefined) {
    return 'unset'
  }
  return record.enabled ? 'on' : 'off'
}

function isDelivered(
  effective: EffectivePluginResponse | undefined,
  skillName: string | undefined,
): boolean {
  if (effective === undefined) {
    return false
  }
  if (skillName === undefined) {
    return true
  }
  return effective.skillNames.includes(skillName)
}

/**
 * Turn a plugin, or one skill inside it, on and off — for everyone, or just for
 * yourself.
 *
 * The two settings sit side by side because that is the rule: a personal record
 * beats the global default in both directions, and clearing it is how you go
 * back to the default rather than pinning it off. The last column says what
 * those two actually add up to for you.
 */
export function PluginActivationTable({
  pluginId,
  pluginName,
}: {
  pluginId: string
  pluginName: string
}) {
  const { isAdmin } = useAuth()
  const { data: skills, isLoading: skillsLoading, isError } = usePluginSkills(pluginId)
  const { data: records, isLoading: recordsLoading } = usePluginActivations(pluginId)
  const { data: effectivePlugins } = useEffectivePlugins()

  const setActivation = useSetActivation({
    onError: (error) => toast.error('Could not save', { description: getErrorMessage(error) }),
  })
  const clearActivation = useClearActivation({
    onError: (error) => toast.error('Could not clear', { description: getErrorMessage(error) }),
  })

  const pending = setActivation.isPending || clearActivation.isPending

  function change(reach: ActivationReach, skillName: string | undefined, setting: Setting) {
    if (setting === 'unset') {
      clearActivation.mutate({
        pluginId,
        params: skillName === undefined ? { reach } : { reach, skillName },
      })
      return
    }
    setActivation.mutate({
      pluginId,
      request: { reach, skillName: skillName ?? null, enabled: setting === 'on' },
    })
  }

  if (skillsLoading || recordsLoading) {
    return (
      <ActivationCard>
        <div className="flex justify-center py-4">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        </div>
      </ActivationCard>
    )
  }

  if (isError) {
    return (
      <ActivationCard>
        <p className="text-sm text-muted-foreground">
          Could not read this plugin's skills. Check that its git URL is reachable.
        </p>
      </ActivationCard>
    )
  }

  const skillList: PluginSkillResponse[] = skills ?? []
  if (skillList.length === 0) {
    return (
      <ActivationCard>
        <p className="text-sm text-muted-foreground">
          This plugin provides no skills, so there is nothing to deliver.
        </p>
      </ActivationCard>
    )
  }

  const rows: Row[] = [
    { skillName: undefined, label: pluginName, description: 'The whole plugin' },
    ...skillList.map((skill) => ({
      skillName: skill.name,
      label: skill.name,
      description: skill.description,
    })),
  ]
  const applied = records ?? []
  const effective = effectivePlugins?.find((p) => p.pluginId === pluginId)

  return (
    <ActivationCard>
      <div className="divide-y">
        <div className="hidden pb-2 text-xs font-medium text-muted-foreground sm:flex sm:items-center sm:gap-4">
          <span className="min-w-0 flex-1">Skill</span>
          <span className={CONTROLS}>
            <span className={CONTROL_WIDTH}>Everyone</span>
            <span className={CONTROL_WIDTH}>You</span>
            <span className={DELIVERED_WIDTH}>You get it</span>
          </span>
        </div>

        {rows.map((row) => (
          <div
            key={row.skillName ?? '__plugin__'}
            className="flex flex-col gap-3 py-3 sm:flex-row sm:items-start sm:gap-4"
          >
            <div className="min-w-0 flex-1">
              <p className="font-mono text-sm break-words">{row.label}</p>
              {row.description === undefined ? null : (
                <p className="text-sm text-muted-foreground break-words">{row.description}</p>
              )}
            </div>
            <div className={CONTROLS}>
              <SettingSelect
                label="Everyone"
                value={settingOf(applied, ActivationReach.everyone, row.skillName)}
                disabled={!isAdmin || pending}
                onChange={(setting) => change(ActivationReach.everyone, row.skillName, setting)}
              />
              <SettingSelect
                label="You"
                value={settingOf(applied, ActivationReach.personal, row.skillName)}
                disabled={pending}
                onChange={(setting) => change(ActivationReach.personal, row.skillName, setting)}
              />
              <span className={`${DELIVERED_WIDTH} shrink-0`}>
                {isDelivered(effective, row.skillName) ? (
                  <Badge variant="secondary">Yes</Badge>
                ) : (
                  <span className="text-sm text-muted-foreground">No</span>
                )}
              </span>
            </div>
          </div>
        ))}
      </div>
      {isAdmin ? null : (
        <p className="mt-4 text-sm text-muted-foreground">
          Only administrators can change what everyone gets by default.
        </p>
      )}
    </ActivationCard>
  )
}

function ActivationCard({ children }: { children: React.ReactNode }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Skills</CardTitle>
        <CardDescription>
          What this plugin provides, and who gets it. "Not set" leaves the default in charge.
        </CardDescription>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  )
}

function SettingSelect({
  label,
  value,
  disabled,
  onChange,
}: {
  label: string
  value: Setting
  disabled: boolean
  onChange: (setting: Setting) => void
}) {
  return (
    <Select value={value} disabled={disabled} onValueChange={(v) => onChange(v as Setting)}>
      {/* The column header is hidden on narrow screens, so the control carries
          its own accessible name rather than relying on a visible one. */}
      <SelectTrigger size="sm" aria-label={label} className={`${CONTROL_WIDTH} shrink-0`}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="on">On</SelectItem>
        <SelectItem value="off">Off</SelectItem>
        <SelectItem value="unset">Not set</SelectItem>
      </SelectContent>
    </Select>
  )
}
