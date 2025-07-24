import type { UpdateRepositoryRequest } from '@moatless/types'
import { ChevronDown, Loader2, Play, Save, Wand2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useDetectFramework } from '@/features/repositories/hooks/use-detect-framework'
import {
  useRepository,
  useRunSetupScript,
  useUpdateRepository,
} from '@/features/repositories/hooks/use-repositories'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper as ConfigurationSection,
  ConfigurationSelect,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Button } from '@/lib/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

interface RepositoryAllConfigSectionProps {
  repositoryId: string
}

// Environment Variables List Component
interface EnvironmentVariablesListProps {
  variables: Record<string, string>
  onChange: (vars: Record<string, string>) => void
}

function EnvironmentVariablesList({ variables, onChange }: EnvironmentVariablesListProps) {
  const [newKey, setNewKey] = useState('')
  const [newValue, setNewValue] = useState('')

  const handleAdd = () => {
    if (newKey && newValue) {
      onChange({ ...variables, [newKey]: newValue })
      setNewKey('')
      setNewValue('')
    }
  }

  const handleRemove = (key: string) => {
    const { [key]: _, ...rest } = variables
    onChange(rest)
  }

  return (
    <div className="space-y-3">
      {Object.entries(variables).map(([key, value]) => (
        <div key={key} className="flex items-center gap-2">
          <Input value={key} disabled className="flex-1" />
          <Input value={value} disabled className="flex-1" />
          <Button type="button" variant="outline" size="sm" onClick={() => handleRemove(key)}>
            Remove
          </Button>
        </div>
      ))}
      <div className="flex items-center gap-2">
        <Input
          placeholder="Key"
          value={newKey}
          onChange={(e) => setNewKey(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && newKey && newValue) {
              e.preventDefault()
              handleAdd()
            }
          }}
          className="flex-1"
        />
        <Input
          placeholder="Value"
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && newKey && newValue) {
              e.preventDefault()
              handleAdd()
            }
          }}
          className="flex-1"
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={handleAdd}
          disabled={!(newKey && newValue)}
        >
          Add
        </Button>
      </div>
    </div>
  )
}

// Collapsible Help Section Component
interface CollapsibleHelpSectionProps {
  title: string
  children: React.ReactNode
}

function CollapsibleHelpSection({ title, children }: CollapsibleHelpSectionProps) {
  const [isOpen, setIsOpen] = useState(false)

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <CollapsibleTrigger asChild>
        <Button variant="ghost" size="sm" className="w-full justify-between">
          {title}
          <ChevronDown className={`h-4 w-4 transition-transform ${isOpen ? 'rotate-180' : ''}`} />
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent className="pt-2">{children}</CollapsibleContent>
    </Collapsible>
  )
}

// Configuration Skeleton Component
function ConfigurationSkeleton() {
  return (
    <div className="space-y-6">
      <div className="space-y-4">
        <div className="h-4 bg-muted rounded w-1/4" />
        <div className="h-10 bg-muted rounded" />
        <div className="h-10 bg-muted rounded" />
      </div>
    </div>
  )
}

export function RepositoryAllConfigSection({ repositoryId }: RepositoryAllConfigSectionProps) {
  const { data: repository, isLoading, error } = useRepository(repositoryId)
  const detectFramework = useDetectFramework(repositoryId)
  const runSetupScript = useRunSetupScript()
  const updateRepository = useUpdateRepository()

  // Derive base form data from repository - following derived state pattern
  const baseFormData = useMemo<Partial<UpdateRepositoryRequest>>(
    () => ({
      name: repository?.name || '',
      description: repository?.description || '',
      path: repository?.path || '',
      useGitWorktrees: repository?.useGitWorktrees || false,
      codeExecution: repository?.codeExecution || { type: 'local' },
      previewConfig: repository?.previewConfig || {},
    }),
    [repository],
  )

  // Track user modifications locally
  const [userModifications, setUserModifications] = useState<Partial<UpdateRepositoryRequest>>({})

  // Merge derived data with user modifications
  const formData = useMemo(
    () => ({
      ...baseFormData,
      ...userModifications,
    }),
    [baseFormData, userModifications],
  )

  // Update a specific field
  const updateField = <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => {
    setUserModifications({
      ...userModifications,
      [field]: value,
    })
  }

  const handleValidateExecution = () => {
    runSetupScript.mutate(repositoryId, {
      onSuccess: () => toast.success('Setup script validation completed'),
      onError: (error) => toast.error(getErrorMessage(error)),
    })
  }

  const handleSave = () => {
    if (Object.keys(userModifications).length === 0) return

    updateRepository.mutate(
      { repoId: repositoryId, request: userModifications },
      {
        onSuccess: () => {
          setUserModifications({})
          toast.success('Repository updated successfully')
        },
        onError: (error) => {
          toast.error('Failed to update repository', {
            description: getErrorMessage(error),
          })
        },
      },
    )
  }

  const handleDiscard = () => {
    setUserModifications({})
    toast.success('Changes discarded')
  }

  const hasChanges = Object.keys(userModifications).length > 0

  const handleDetectFramework = async () => {
    try {
      const detected = await detectFramework.mutateAsync()
      if (detected) {
        updateField('previewConfig', detected)
        toast.success('Framework detected and will be saved')
      }
    } catch (error) {
      toast.error(getErrorMessage(error))
    }
  }

  if (isLoading) {
    return <ConfigurationSkeleton />
  }

  if (error || !repository) {
    return (
      <div className="text-center text-destructive">Failed to load repository configuration</div>
    )
  }

  return (
    <ConfigurationSection>
      {/* General Settings */}
      <ConfigurationGroup title="General Settings">
        <ConfigurationItem label="Repository Name" description="Display name for your repository">
          <ConfigurationInput
            value={formData.name || ''}
            onChange={(e) => updateField('name', e.target.value)}
          />
        </ConfigurationItem>

        <ConfigurationItem
          label="Description"
          description="Optional description of this repository"
        >
          <ConfigurationTextarea
            value={formData.description || ''}
            onChange={(e) => updateField('description', e.target.value)}
            rows={3}
          />
        </ConfigurationItem>

        <ConfigurationItem label="Repository Path" description="Local path to your repository">
          <ConfigurationInput
            value={formData.path || ''}
            onChange={(e) => updateField('path', e.target.value)}
          />
        </ConfigurationItem>

        <ConfigurationItem
          label="Git Worktrees"
          description="Use git worktrees for isolated environments"
        >
          <ConfigurationSwitch
            checked={formData.useGitWorktrees || false}
            onCheckedChange={(checked) => updateField('useGitWorktrees', checked)}
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      {/* Code Execution Settings */}
      <ConfigurationGroup title="Code Execution" showSeparator>
        <ConfigurationItem
          label="Execution Type"
          description="Choose how to run code in this repository"
        >
          <ConfigurationSelect
            value={formData.codeExecution?.type || 'local'}
            onValueChange={(value) =>
              updateField('codeExecution', {
                ...formData.codeExecution,
                type: value as 'local' | 'docker',
              })
            }
            options={[
              { value: 'local', label: 'Local' },
              { value: 'docker', label: 'Docker' },
            ]}
          />
        </ConfigurationItem>

        {formData.codeExecution?.type === 'docker' && (
          <ConfigurationItem
            label="Docker Image"
            description="Docker image to use for code execution"
          >
            <ConfigurationInput
              value={formData.codeExecution.dockerImage || ''}
              onChange={(e) =>
                updateField('codeExecution', {
                  type: formData.codeExecution?.type || 'local',
                  ...formData.codeExecution,
                  dockerImage: e.target.value,
                })
              }
              placeholder="e.g., node:20-alpine"
            />
          </ConfigurationItem>
        )}

        <ConfigurationItem
          label="Setup Commands"
          description="Commands to run before executing code"
        >
          <div className="space-y-2">
            <ConfigurationTextarea
              value={formData.codeExecution?.setupCommands?.join('\n') || ''}
              onChange={(e) =>
                updateField('codeExecution', {
                  type: formData.codeExecution?.type || 'local',
                  ...formData.codeExecution,
                  setupCommands: e.target.value.split('\n').filter(Boolean),
                })
              }
              placeholder="npm install&#10;npm run build"
              rows={4}
            />
            <Button
              size="sm"
              variant="outline"
              onClick={handleValidateExecution}
              disabled={runSetupScript.isPending}
            >
              <Play className="mr-2 h-4 w-4" />
              {runSetupScript.isPending ? 'Validating...' : 'Validate'}
            </Button>
          </div>
        </ConfigurationItem>

        <ConfigurationItem
          label="Environment Variables"
          description="Variables available during code execution"
        >
          <EnvironmentVariablesList
            variables={formData.codeExecution?.envVars || {}}
            onChange={(vars) =>
              updateField('codeExecution', {
                type: formData.codeExecution?.type || 'local',
                ...formData.codeExecution,
                envVars: vars,
              })
            }
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      {/* Preview Environment Settings */}
      <ConfigurationGroup title="Preview Environment" showSeparator>
        <ConfigurationItem label="Framework" description="Web framework used in this repository">
          <div className="flex gap-2">
            <ConfigurationSelect
              value={formData.previewConfig?.framework || 'unknown'}
              onValueChange={(value) =>
                updateField('previewConfig', {
                  ...formData.previewConfig,
                  framework: value as 'vite' | 'nextjs' | 'cra' | 'webpack' | 'unknown',
                })
              }
              options={[
                { value: 'nextjs', label: 'Next.js' },
                { value: 'vite', label: 'Vite' },
                { value: 'cra', label: 'Create React App' },
                { value: 'webpack', label: 'Webpack' },
                { value: 'unknown', label: 'Unknown' },
              ]}
            />
            <Button
              size="sm"
              variant="outline"
              onClick={handleDetectFramework}
              disabled={detectFramework.isPending}
            >
              <Wand2 className="mr-2 h-4 w-4" />
              {detectFramework.isPending ? 'Detecting...' : 'Auto-detect'}
            </Button>
          </div>
        </ConfigurationItem>

        <ConfigurationItem
          label="Framework Version"
          description="Version of the framework (optional)"
        >
          <ConfigurationInput
            value={formData.previewConfig?.frameworkVersion || ''}
            onChange={(e) =>
              updateField('previewConfig', {
                ...formData.previewConfig,
                frameworkVersion: e.target.value,
              })
            }
            placeholder="e.g., 14.0.0"
          />
        </ConfigurationItem>

        <ConfigurationItem label="Start Command" description="Command to start the preview server">
          <ConfigurationInput
            value={formData.previewConfig?.serverStartCommand || ''}
            onChange={(e) =>
              updateField('previewConfig', {
                ...formData.previewConfig,
                serverStartCommand: e.target.value,
              })
            }
            placeholder="e.g., npm run dev"
          />
        </ConfigurationItem>

        <ConfigurationItem label="Script Injection" description="How to inject the preview script">
          <ConfigurationSelect
            value={formData.previewConfig?.injectionStrategy?.method || 'html-injection'}
            onValueChange={(value) =>
              updateField('previewConfig', {
                ...formData.previewConfig,
                injectionStrategy: {
                  method: value as 'html-injection' | 'middleware',
                  config: {},
                },
              })
            }
            options={[
              { value: 'html-injection', label: 'HTML Injection' },
              { value: 'middleware', label: 'Middleware' },
            ]}
          />
        </ConfigurationItem>

        {formData.previewConfig?.injectionStrategy?.method === 'html-injection' && (
          <ConfigurationItem label="HTML File Path" description="Path to the main HTML file">
            <ConfigurationInput
              value={formData.previewConfig?.injectionStrategy?.config?.htmlFilePath || ''}
              onChange={(e) =>
                updateField('previewConfig', {
                  ...formData.previewConfig,
                  injectionStrategy: {
                    method: formData.previewConfig?.injectionStrategy?.method || 'html-injection',
                    config: {
                      ...formData.previewConfig?.injectionStrategy?.config,
                      htmlFilePath: e.target.value,
                    },
                  },
                })
              }
              placeholder="e.g., public/index.html"
            />
          </ConfigurationItem>
        )}

        <CollapsibleHelpSection title="Learn More">
          <div className="space-y-2 text-sm text-muted-foreground">
            <p>
              The preview environment allows you to see changes in real-time by running your
              application and injecting preview scripts.
            </p>
            <p>
              <strong>HTML Injection:</strong> Automatically modifies your HTML files to include
              preview scripts.
            </p>
            <p>
              <strong>Middleware:</strong> Uses custom middleware to inject scripts at runtime.
            </p>
          </div>
        </CollapsibleHelpSection>
      </ConfigurationGroup>

      {/* Save/Discard buttons at section level */}
      <div className="flex justify-end gap-3 pt-6 border-t">
        <Button type="button" variant="outline" onClick={handleDiscard} disabled={!hasChanges}>
          Discard Changes
        </Button>
        <Button
          type="button"
          onClick={handleSave}
          disabled={!hasChanges || updateRepository.isPending}
        >
          {updateRepository.isPending ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Saving...
            </>
          ) : (
            <>
              <Save className="mr-2 h-4 w-4" />
              Save Changes
            </>
          )}
        </Button>
      </div>
    </ConfigurationSection>
  )
}
