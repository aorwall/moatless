import type { PluginSkillResponse } from '@moatless/api-client'
import { ActivationReach } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  type ActivationRow,
  activationRows,
  CONTROL_WIDTH,
  CONTROLS,
  DELIVERED_WIDTH,
  isDelivered,
  type Setting,
  SettingSelect,
  settingOf,
} from '@/features/plugins/components/activation-controls'
import {
  useClearActivation,
  useEffectivePlugins,
  usePluginActivations,
  usePluginSkills,
  useSetActivation,
} from '@/features/plugins/hooks/use-plugins'
import { Badge } from '@/lib/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { getErrorMessage } from '@/lib/utils/error'

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

  const rows: ActivationRow[] = activationRows(pluginName, skillList)
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
