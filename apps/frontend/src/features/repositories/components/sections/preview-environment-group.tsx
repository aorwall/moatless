import type { UpdateRepositoryRequest } from '@moatless/types'
import { CollapsibleHelpSection } from '@/lib/components/collapsible-help-section'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'

interface PreviewEnvironmentGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

export function PreviewEnvironmentGroup({ formData, onUpdateField }: PreviewEnvironmentGroupProps) {
  const updateField = onUpdateField

  return (
    <ConfigurationGroup title="Preview Environment" showSeparator>
      <ConfigurationItem
        label="Start Commands"
        description="Commands to start the preview server (one per line)"
      >
        <ConfigurationTextarea
          value={formData.previewConfig?.serverStartCommands?.join('\n') || ''}
          onChange={(e) => {
            const commands = e.target.value.split('\n')
            updateField('previewConfig', {
              ...formData.previewConfig,
              serverStartCommands: commands,
              enableInspector: formData.previewConfig?.enableInspector ?? true,
            })
          }}
          placeholder="npm install&#10;npm run dev"
          rows={3}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Build Command"
        description="Command to build the project for production deployment"
      >
        <ConfigurationInput
          value={formData.previewConfig?.buildCommand || ''}
          onChange={(e) =>
            updateField('previewConfig', {
              ...formData.previewConfig,
              buildCommand: e.target.value,
              enableInspector: formData.previewConfig?.enableInspector ?? true,
            })
          }
          placeholder="e.g., npm run build"
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Startup Timeout"
        description="Maximum time to wait for preview server to become HTTP-responsive (in seconds, default: 30)"
      >
        <ConfigurationInput
          type="number"
          min={5}
          value={formData.previewConfig?.startupTimeoutSeconds ?? ''}
          onChange={(e) => {
            const value = e.target.value ? Number(e.target.value) : undefined
            updateField('previewConfig', {
              ...formData.previewConfig,
              startupTimeoutSeconds: value,
              enableInspector: formData.previewConfig?.enableInspector ?? true,
            })
          }}
          placeholder="30"
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Take Screenshots"
        description="Automatically take screenshots after each task iteration"
      >
        <ConfigurationSwitch
          checked={formData.previewConfig?.takeScreenshots}
          onCheckedChange={(checked) =>
            updateField('previewConfig', {
              ...formData.previewConfig,
              takeScreenshots: checked,
              enableInspector: formData.previewConfig?.enableInspector ?? true,
            })
          }
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Enable Inspector"
        description="Inject inspector script for element selection and debugging (may cause compatibility issues with some frameworks)"
      >
        <ConfigurationSwitch
          checked={formData.previewConfig?.enableInspector !== false}
          onCheckedChange={(checked) =>
            updateField('previewConfig', {
              ...formData.previewConfig,
              enableInspector: checked,
            })
          }
        />
      </ConfigurationItem>

      <CollapsibleHelpSection title="Learn More">
        <div className="space-y-2 text-sm text-muted-foreground">
          <p>
            The preview environment allows you to see changes in real-time by running your
            application and optionally injecting inspector scripts.
          </p>
          <p>
            <strong>Inspector Script:</strong> When enabled, injects scripts that allow element
            selection and debugging. This uses a proxy server to modify your HTML at runtime.
          </p>
          <p>
            <strong>Direct Preview:</strong> When inspector is disabled, serves your application
            directly without any modifications. Use this if the inspector causes compatibility
            issues.
          </p>
        </div>
      </CollapsibleHelpSection>
    </ConfigurationGroup>
  )
}
