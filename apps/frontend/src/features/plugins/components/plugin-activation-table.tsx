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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
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
 * The two settings are shown side by side because that is the rule: a personal
 * record beats the global default in both directions, and clearing it is how
 * you go back to the default rather than pinning it off. The last column says
 * what those two actually add up to for you.
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
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Skill</TableHead>
            <TableHead className="w-36">Everyone</TableHead>
            <TableHead className="w-36">You</TableHead>
            <TableHead className="w-28">You get it</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row) => (
            <TableRow key={row.skillName ?? '__plugin__'}>
              <TableCell>
                <p className="font-mono text-sm">{row.label}</p>
                {row.description === undefined ? null : (
                  <p className="text-sm text-muted-foreground">{row.description}</p>
                )}
              </TableCell>
              <TableCell>
                <SettingSelect
                  value={settingOf(applied, ActivationReach.everyone, row.skillName)}
                  disabled={!isAdmin || pending}
                  onChange={(setting) => change(ActivationReach.everyone, row.skillName, setting)}
                />
              </TableCell>
              <TableCell>
                <SettingSelect
                  value={settingOf(applied, ActivationReach.personal, row.skillName)}
                  disabled={pending}
                  onChange={(setting) => change(ActivationReach.personal, row.skillName, setting)}
                />
              </TableCell>
              <TableCell>
                {isDelivered(effective, row.skillName) ? (
                  <Badge variant="secondary">Yes</Badge>
                ) : (
                  <span className="text-sm text-muted-foreground">No</span>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
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
  value,
  disabled,
  onChange,
}: {
  value: Setting
  disabled: boolean
  onChange: (setting: Setting) => void
}) {
  return (
    <Select value={value} disabled={disabled} onValueChange={(v) => onChange(v as Setting)}>
      <SelectTrigger size="sm">
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
