import type { TransformOptions } from '@moatless/figma'
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Label } from '@/lib/components/ui/label'
import { cn } from '@/lib/utils'

interface TransformOptionsPanelProps {
  options: TransformOptions
  onChange: (options: TransformOptions) => void
}

interface OptionDefinition {
  key: keyof TransformOptions
  label: string
  description: string
  defaultValue: boolean
  indent?: number
  dependsOn?: keyof TransformOptions
}

const OPTION_GROUPS: Record<string, OptionDefinition[]> = {
  'Instance Expansion': [
    {
      key: 'expandInstances' as const,
      label: 'Expand Instances',
      description:
        '🔧 Pipeline step 1: Replaces INSTANCE nodes with their component definitions. Looks up symbols from "Internal Only Canvas" and recursively expands nested instances.',
      defaultValue: true,
      indent: 0,
    },
    {
      key: 'applyInstanceOverrides' as const,
      label: 'Apply Instance Overrides',
      description:
        '⚙️ Requires "Expand Instances": Applies instance-specific customizations (text overrides, visibility changes). Only effective when parent option is enabled.',
      defaultValue: false,
      indent: 1,
      dependsOn: 'expandInstances',
    },
    {
      key: 'applyComponentProps' as const,
      label: 'Apply Component Props',
      description:
        '⚙️ Requires "Expand Instances": Applies component property assignments (TEXT and BOOLEAN overrides). Resolves defID references to apply custom text content and visibility states.',
      defaultValue: true,
      indent: 1,
      dependsOn: 'expandInstances',
    },
    {
      key: 'filterHidden' as const,
      label: 'Filter Hidden Nodes',
      description:
        '🔧 Pipeline step 2: Removes nodes with visible:false after instance expansion. Cleans up hidden elements from symbol definitions.',
      defaultValue: true,
      indent: 0,
    },
  ],
  'Output Control': [
    {
      key: 'includeNodeStyle' as const,
      label: 'Include Node Styles',
      description:
        '🎨 Controls node.style output: Include background, border, radius, shadow, blur, and opacity. Disable to see only structure without decorative styling.',
      defaultValue: true,
      indent: 0,
    },
    {
      key: 'includeTextStyle' as const,
      label: 'Include Text Styles',
      description:
        '✍️ Controls text.style output: Include fontFamily, fontSize, fontWeight, and color. Disable to see text content without typography details.',
      defaultValue: true,
      indent: 0,
    },
    {
      key: 'includeLayoutDimensions' as const,
      label: 'Include Layout Dimensions',
      description:
        '📏 Controls layout dimensions: Include gap, padding, width, and height values. Structural properties (mode, alignment) always included.',
      defaultValue: true,
      indent: 0,
    },
    {
      key: 'includeMetadata' as const,
      label: 'Include Metadata',
      description:
        '🏷️ Controls metadata output: Include version, source, id, and meta fields. Disable for minimal output focused on structure and styling only.',
      defaultValue: false,
      indent: 0,
    },
  ],
  'Structure Detection & Cleanup': [
    {
      key: 'detectStructuralFrames' as const,
      label: 'Detect Structural Frames',
      description:
        '✨ Pipeline step 3: Identifies organizational frames with meta.structural:true. These frames are for organization only and have styling omitted.',
      defaultValue: true,
      indent: 0,
    },
    {
      key: 'flattenStructuralWrappers' as const,
      label: 'Flatten Structural Wrappers',
      description:
        '✨ Requires "Detect Structural Frames": Removes single-child structural frames by merging layout into child. Reduces unnecessary nesting.',
      defaultValue: true,
      indent: 1,
      dependsOn: 'detectStructuralFrames',
    },
    {
      key: 'sortChildrenByPosition' as const,
      label: 'Sort Children by Position',
      description:
        '🔧 Pipeline step 4: Sorts children by visual position (LTR for horizontal, TTB for vertical). Critical for correct CSS flexbox rendering order.',
      defaultValue: true,
      indent: 0,
    },
  ],
  'Sizing & Dimensions': [
    {
      key: 'useFlexibleDimensions' as const,
      label: 'Use Flexible Dimensions',
      description:
        '✨ Dimension transformation: Converts fixed widths to flexible (auto/100%) for containers with sizingIntent:fixed. Prevents viewport overflow.',
      defaultValue: true,
      indent: 0,
    },
  ],
  Formatting: [
    {
      key: 'includePositionCoords' as const,
      label: 'Include Position Coords',
      description:
        '⚠️ Usually disabled: Includes X/Y coordinates for auto-layout children. Not needed for CSS flexbox. Only useful for absolute positioning.',
      defaultValue: false,
      indent: 0,
    },
    {
      key: 'detectIcons' as const,
      label: 'Detect Icons',
      description:
        '🎨 Icon detection: Detects icon components and adds meta.iconType and meta.iconDescription. Prevents incorrect styling suggestions.',
      defaultValue: true,
      indent: 0,
    },
  ],
}

/**
 * Transform Options Panel Component
 * Checkboxes for configuring TransformOptions
 */
export function TransformOptionsPanel({ options, onChange }: TransformOptionsPanelProps) {
  const handleToggle = (key: keyof TransformOptions, checked: boolean) => {
    onChange({
      ...options,
      [key]: checked,
    })
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="text-sm font-medium">Transform Options</div>

      {Object.entries(OPTION_GROUPS).map(([groupName, groupOptions]) => (
        <div key={groupName} className="flex flex-col gap-3">
          <div className="text-xs font-medium text-muted-foreground">{groupName}</div>
          <div className="space-y-3">
            {groupOptions.map((option) => {
              const isDisabled = option.dependsOn && !options[option.dependsOn]
              return (
                <div
                  key={option.key}
                  className="flex items-start gap-3"
                  style={{ paddingLeft: `${(option.indent ?? 0) * 24}px` }}
                >
                  <Checkbox
                    id={option.key}
                    checked={options[option.key] ?? option.defaultValue}
                    onCheckedChange={(checked) => handleToggle(option.key, checked as boolean)}
                    className="mt-0.5"
                    disabled={isDisabled}
                  />
                  <div className="flex-1">
                    <Label
                      htmlFor={option.key}
                      className={cn(
                        'cursor-pointer text-sm font-normal leading-none',
                        isDisabled && 'text-muted-foreground',
                      )}
                    >
                      {option.label}
                    </Label>
                    <p className="mt-1 text-xs text-muted-foreground">{option.description}</p>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      ))}
    </div>
  )
}
