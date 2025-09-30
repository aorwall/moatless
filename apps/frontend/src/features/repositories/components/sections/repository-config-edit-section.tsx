import type { UpdateRepositoryRequest } from '@moatless/types'
import { ChevronDown, Loader2, Play, Save, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { CommandExecutionDialog } from '@/features/repositories/components/command-execution-dialog'
import {
  useDeleteRepository,
  useRepository,
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
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/lib/components/ui/alert-dialog'
import { Button } from '@/lib/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

interface RepositoryConfigEditSectionProps {
  repositoryId: string
  onDeleted?: () => void
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

export function RepositoryConfigEditSection({
  repositoryId,
  onDeleted,
}: RepositoryConfigEditSectionProps) {
  const { data: repository, isLoading, error } = useRepository(repositoryId)
  const updateRepository = useUpdateRepository()
  const deleteRepository = useDeleteRepository()
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const [showCommandDialog, setShowCommandDialog] = useState(false)

  // Derive base form data from repository - following derived state pattern
  const baseFormData = useMemo<Partial<UpdateRepositoryRequest>>(
    () => ({
      name: repository?.name || '',
      description: repository?.description || '',
      path: repository?.path || '',
      useGitWorktrees: repository?.useGitWorktrees || false,
      cloneOnNewTask: repository?.cloneOnNewTask || false,
      codeExecution: repository?.codeExecution || { type: 'local' },
      previewConfig: {
        enableInspector: true,
        ...repository?.previewConfig,
      },
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
    setUserModifications((prev) => ({
      ...prev,
      [field]: value,
    }))
  }

  const handleValidateExecution = () => {
    if (!repository || !formData.codeExecution?.setupCommands) return

    const commands = formData.codeExecution.setupCommands

    if (commands.length === 0) {
      toast.error('No setup commands to validate')
      return
    }

    setShowCommandDialog(true)
  }

  const handleCommandExecutionComplete = (success: boolean) => {
    if (success) {
      toast.success('Setup validation completed successfully')
    } else {
      toast.error('Setup validation failed - check the output for errors')
    }
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

  const handleDelete = () => {
    deleteRepository.mutate(repositoryId, {
      onSuccess: () => {
        toast.success('Repository deleted successfully')
        setShowDeleteDialog(false)
        onDeleted?.()
      },
      onError: (error) => {
        toast.error('Failed to delete repository', {
          description: getErrorMessage(error),
        })
        setShowDeleteDialog(false)
      },
    })
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

        {repository?.remoteUrl && (
          <ConfigurationItem label="GitHub URL" description="Remote repository URL (read-only)">
            <ConfigurationInput value={repository.remoteUrl} disabled className="bg-muted" />
          </ConfigurationItem>
        )}

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
            onCheckedChange={(checked) => {
              updateField('useGitWorktrees', checked)
              // Disable cloneOnNewTask when worktrees are enabled
              if (checked) {
                updateField('cloneOnNewTask', false)
              }
            }}
          />
        </ConfigurationItem>

        <ConfigurationItem
          label="Clone on New Task"
          description="Clone the entire repository for each new task instead of using git worktrees. Provides complete isolation between tasks."
        >
          <ConfigurationSwitch
            checked={formData.cloneOnNewTask || false}
            onCheckedChange={(checked) => {
              updateField('cloneOnNewTask', checked)
              // Disable worktrees when cloning is enabled
              if (checked) {
                updateField('useGitWorktrees', false)
              }
            }}
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
                  setupCommands: e.target.value.split('\n'),
                })
              }
              placeholder="npm install&#10;npm run build"
              rows={4}
            />
            <Button
              size="sm"
              variant="outline"
              onClick={handleValidateExecution}
              disabled={!formData.codeExecution?.setupCommands?.length}
            >
              <Play className="mr-2 h-4 w-4" />
              Validate
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
            checked={formData.previewConfig?.takeScreenshots || false}
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

      {/* Save/Discard/Delete buttons at section level */}
      <div className="flex justify-between gap-3 pt-6 border-t">
        <Button
          type="button"
          variant="destructive"
          onClick={() => setShowDeleteDialog(true)}
          disabled={deleteRepository.isPending}
        >
          {deleteRepository.isPending ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Deleting...
            </>
          ) : (
            <>
              <Trash2 className="mr-2 h-4 w-4" />
              Delete Repository
            </>
          )}
        </Button>
        <div className="flex gap-3">
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
      </div>

      {/* Delete Confirmation Dialog */}
      <AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Repository</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{repository?.name}"? This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteRepository.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDelete}
              disabled={deleteRepository.isPending}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteRepository.isPending ? 'Deleting...' : 'Delete'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Command Execution Dialog */}
      {formData.codeExecution?.setupCommands && (
        <CommandExecutionDialog
          repositoryId={repositoryId}
          commands={formData.codeExecution.setupCommands}
          title="Validating Setup Commands"
          description="Running your setup commands in the repository workspace to verify they work correctly."
          open={showCommandDialog}
          onOpenChange={setShowCommandDialog}
          onComplete={handleCommandExecutionComplete}
        />
      )}
    </ConfigurationSection>
  )
}
