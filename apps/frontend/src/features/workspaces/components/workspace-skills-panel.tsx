import { ActivationReach } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import {
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
  usePlugins,
  useSetActivation,
} from '@/features/plugins/hooks/use-plugins'
import { Badge } from '@/lib/components/ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { getErrorMessage } from '@/lib/utils/error'

/**
 * Which skills load in this workspace, plugin by plugin.
 *
 * A workspace record outranks both the deployment-wide default and anyone's own
 * setting, so this is where a workspace adds a skill nobody turned on and where
 * it keeps out one everybody has. "Not set" writes no record and leaves those
 * lower settings in charge; the last column says what it all comes to.
 */
export function WorkspaceSkillsPanel({
  workspaceId,
  readOnly = false,
}: {
  workspaceId: string
  readOnly?: boolean
}) {
  const { data: plugins, isLoading, isError } = usePlugins()

  if (isLoading) {
    return (
      <SkillsCard>
        <div className="flex justify-center py-4">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        </div>
      </SkillsCard>
    )
  }

  if (isError) {
    return (
      <SkillsCard>
        <p className="text-sm text-muted-foreground">Could not read the plugin registry.</p>
      </SkillsCard>
    )
  }

  const pluginList = plugins ?? []
  if (pluginList.length === 0) {
    return (
      <SkillsCard>
        <p className="text-sm text-muted-foreground">
          No plugins are registered, so there is nothing for this workspace to narrow. Skills
          committed in a repository's own <code>.claude/skills</code> always load.
        </p>
      </SkillsCard>
    )
  }

  return (
    <SkillsCard>
      <div className="space-y-6">
        {pluginList.map((plugin) => (
          <PluginRows
            key={plugin.id}
            pluginId={plugin.id}
            pluginName={plugin.name}
            workspaceId={workspaceId}
            readOnly={readOnly}
          />
        ))}
      </div>
    </SkillsCard>
  )
}

/**
 * One plugin's rows. Split out so each plugin's skills load on their own — the
 * read syncs the plugin from its source, and one unreachable git URL should not
 * hold up the rest of the list.
 */
function PluginRows({
  pluginId,
  pluginName,
  workspaceId,
  readOnly,
}: {
  pluginId: string
  pluginName: string
  workspaceId: string
  readOnly: boolean
}) {
  const { data: skills, isLoading, isError } = usePluginSkills(pluginId)
  const { data: records } = usePluginActivations(pluginId, workspaceId)
  const { data: effectivePlugins } = useEffectivePlugins(workspaceId)

  const setActivation = useSetActivation({
    onError: (error) => toast.error('Could not save', { description: getErrorMessage(error) }),
  })
  const clearActivation = useClearActivation({
    onError: (error) => toast.error('Could not clear', { description: getErrorMessage(error) }),
  })

  const pending = setActivation.isPending || clearActivation.isPending

  function change(skillName: string | undefined, setting: Setting) {
    const reach = ActivationReach.workspace
    if (setting === 'unset') {
      clearActivation.mutate({
        pluginId,
        params:
          skillName === undefined ? { reach, workspaceId } : { reach, workspaceId, skillName },
      })
      return
    }
    setActivation.mutate({
      pluginId,
      request: { reach, workspaceId, skillName: skillName ?? null, enabled: setting === 'on' },
    })
  }

  if (isLoading) {
    return (
      <PluginBlock name={pluginName}>
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      </PluginBlock>
    )
  }

  if (isError) {
    return (
      <PluginBlock name={pluginName}>
        <p className="text-sm text-muted-foreground">
          Could not read this plugin's skills. Check that its git URL is reachable.
        </p>
      </PluginBlock>
    )
  }

  const skillList = skills ?? []
  if (skillList.length === 0) {
    return (
      <PluginBlock name={pluginName}>
        <p className="text-sm text-muted-foreground">This plugin provides no skills.</p>
      </PluginBlock>
    )
  }

  const rows = activationRows(pluginName, skillList)
  const applied = records ?? []
  const effective = effectivePlugins?.find((p) => p.pluginId === pluginId)

  return (
    <PluginBlock name={pluginName}>
      <div className="divide-y">
        <div className="hidden pb-2 text-xs font-medium text-muted-foreground sm:flex sm:items-center sm:gap-4">
          <span className="min-w-0 flex-1">Skill</span>
          <span className={CONTROLS}>
            <span className={CONTROL_WIDTH}>This workspace</span>
            <span className={DELIVERED_WIDTH}>Loads here</span>
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
                label={`${row.label} in this workspace`}
                value={settingOf(applied, ActivationReach.workspace, row.skillName)}
                disabled={readOnly || pending}
                onChange={(setting) => change(row.skillName, setting)}
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
    </PluginBlock>
  )
}

function PluginBlock({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="pb-1 text-sm font-medium">{name}</p>
      {children}
    </div>
  )
}

function SkillsCard({ children }: { children: React.ReactNode }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Skills</CardTitle>
        <CardDescription>
          Which plugin skills load for tasks in this workspace. What is set here wins over both the
          deployment default and anyone's own setting. "Not set" leaves those in charge.
        </CardDescription>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  )
}
