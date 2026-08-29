import type {
  ActivationReach,
  ActivationResponse,
  EffectivePluginResponse,
} from '@moatless/api-client'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

/**
 * The pieces every activation control is built from, shared by the admin
 * plugin page (everyone / you) and the workspace page (this workspace).
 *
 * They read the same records and write through the same endpoint, so the rules
 * about what "not set" means and what counts as delivered live here once.
 */

/** What one select can say. `unset` means "no record" — not the same as off. */
export const SETTINGS = ['on', 'off', 'unset'] as const
export type Setting = (typeof SETTINGS)[number]

/**
 * The controls block, sized once so the header labels line up with the rows
 * beneath them without a table. Each control is fixed-width and never shrinks —
 * a long skill description wraps in the space left over rather than pushing the
 * controls off the right-hand edge.
 */
export const CONTROLS = 'flex shrink-0 items-center gap-3'
export const CONTROL_WIDTH = 'w-32'
export const DELIVERED_WIDTH = 'w-20'

/** One row: the plugin as a whole, or one skill inside it. */
export type ActivationRow = {
  /** `undefined` means the whole plugin. */
  skillName: string | undefined
  label: string
  description: string | undefined
}

export function settingOf(
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

export function isDelivered(
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

export function SettingSelect({
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

/** The plugin-as-a-whole row followed by one row per skill, in that order. */
export function activationRows(
  pluginName: string,
  skills: { name: string; description: string }[],
): ActivationRow[] {
  return [
    { skillName: undefined, label: pluginName, description: 'The whole plugin' },
    ...skills.map((skill) => ({
      skillName: skill.name,
      label: skill.name,
      description: skill.description,
    })),
  ]
}
