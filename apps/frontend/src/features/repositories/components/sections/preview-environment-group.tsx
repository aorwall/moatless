import type { UpdateRepositoryRequest } from '@moatless/api-client'
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

  const serverStartCommands = formData.serverStartCommands ?? []

  return (
    <ConfigurationGroup title="Preview Environment" showSeparator>
      <ConfigurationItem
        label="Start Commands"
        description="Commands to start the preview server (one per line)"
      >
        <ConfigurationTextarea
          value={serverStartCommands.join('\n')}
          onChange={(e) => {
            const commands = e.target.value.split('\n')
            updateField('serverStartCommands', commands)
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
          value={formData.buildCommand || ''}
          onChange={(e) => updateField('buildCommand', e.target.value || null)}
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
          value={formData.startupTimeoutSeconds ?? ''}
          onChange={(e) => {
            const value = e.target.value ? Number(e.target.value) : null
            updateField('startupTimeoutSeconds', value)
          }}
          placeholder="30"
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Auto-Reload"
        description="Server has built-in auto-reload/watch functionality. When enabled, the preview server won't restart on code changes since changes are picked up automatically."
      >
        <ConfigurationSwitch
          checked={formData.autoReload ?? false}
          onCheckedChange={(checked) => updateField('autoReload', checked)}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Enable Inspector"
        description="Inject inspector script for element selection and debugging (may cause compatibility issues with some frameworks)"
      >
        <ConfigurationSwitch
          checked={formData.enableInspector !== false}
          onCheckedChange={(checked) => updateField('enableInspector', checked)}
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
