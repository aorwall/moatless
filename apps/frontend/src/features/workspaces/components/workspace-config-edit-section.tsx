import type { UpdateWorkspaceRequest } from '@moatless/api-client'
import { GitBranch, Loader2, Save, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  AgentConfigurationGroup,
  CodeExecutionGroup,
  PluginsGroup,
  ServersGroup,
} from '@/features/run-config'
import {
  useDeleteWorkspace,
  useUpdateWorkspace,
  useWorkspace,
} from '@/features/workspaces/hooks/use-workspaces'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper as ConfigurationSection,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
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
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'
import { WorkspaceReposPanel } from './workspace-repos-panel'

interface WorkspaceConfigEditSectionProps {
  workspaceId: string
  onDeleted?: () => void
}

function ConfigurationSkeleton() {
  return (
    <div className="space-y-6">
      <div className="space-y-4">
        <div className="h-4 w-1/4 rounded bg-muted" />
        <div className="h-10 rounded bg-muted" />
        <div className="h-10 rounded bg-muted" />
      </div>
    </div>
  )
}

export function WorkspaceConfigEditSection({
  workspaceId,
  onDeleted,
}: WorkspaceConfigEditSectionProps) {
  const { data: workspace, isLoading, error } = useWorkspace(workspaceId)
  const updateWorkspace = useUpdateWorkspace()
  const deleteWorkspace = useDeleteWorkspace()
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)

  // Derive base form data from the workspace. `UpdateWorkspaceRequest` is
  // all-optional, so using it directly (not `Partial<>`) keeps value types free
  // of the `| undefined` widening that would otherwise need a payload cast.
  const baseFormData = useMemo<UpdateWorkspaceRequest>(() => {
    if (!workspace) {
      return {}
    }
    return {
      name: workspace.name || '',
      description: workspace.description || '',
      systemPrompt: workspace.systemPrompt || '',
      ...(workspace.dockerImage !== undefined && { dockerImage: workspace.dockerImage }),
      ...(workspace.setupCommands !== undefined && { setupCommands: workspace.setupCommands }),
      ...(workspace.envVars !== undefined && { envVars: workspace.envVars }),
      ...(workspace.servers !== undefined && { servers: workspace.servers }),
      ...(workspace.plugins !== undefined && { plugins: workspace.plugins }),
    }
  }, [workspace])

  const [userModifications, setUserModifications] = useState<UpdateWorkspaceRequest>({})

  const formData = useMemo(
    () => ({ ...baseFormData, ...userModifications }),
    [baseFormData, userModifications],
  )

  const updateField = <K extends keyof UpdateWorkspaceRequest>(
    field: K,
    value: UpdateWorkspaceRequest[K],
  ) => {
    setUserModifications((prev) => ({ ...prev, [field]: value }))
  }

  const hasChanges = Object.keys(userModifications).length > 0

  const handleSave = () => {
    if (!hasChanges) {
      return
    }
    // Empty strings signal field removal (sent as null).
    const payload: UpdateWorkspaceRequest = {
      ...formData,
      description: formData.description?.trim() || null,
      systemPrompt: formData.systemPrompt?.trim() || null,
    }
    updateWorkspace.mutate(
      { workspaceId, request: payload },
      {
        onSuccess: () => {
          setUserModifications({})
          toast.success('Workspace updated successfully')
        },
        onError: (err) =>
          toast.error('Failed to update workspace', { description: getErrorMessage(err) }),
      },
    )
  }

  const handleDiscard = () => {
    setUserModifications({})
    toast.success('Changes discarded')
  }

  const handleDelete = () => {
    deleteWorkspace.mutate(workspaceId, {
      onSuccess: () => {
        toast.success('Workspace deleted successfully')
        setShowDeleteDialog(false)
        onDeleted?.()
      },
      onError: (err) => {
        toast.error('Failed to delete workspace', { description: getErrorMessage(err) })
        setShowDeleteDialog(false)
      },
    })
  }

  if (isLoading) {
    return <ConfigurationSkeleton />
  }

  if (error || !workspace) {
    return (
      <div className="text-center text-destructive">Failed to load workspace configuration</div>
    )
  }

  // Git-synced workspaces are defined in git and read-only; the write API 409s.
  const isGitSynced = workspace.syncedFromGit ?? false

  return (
    <ConfigurationSection>
      {isGitSynced && (
        <Alert>
          <GitBranch className="h-4 w-4" />
          <AlertTitle>Synced from Git</AlertTitle>
          <AlertDescription>
            This workspace is defined in .moatless/workspaces.json and is read-only. Edit the file
            in Git to change it.
          </AlertDescription>
        </Alert>
      )}

      <ConfigurationGroup title="General Settings">
        <div className="flex items-center gap-2">
          <Badge variant="outline" className="capitalize">
            {workspace.kind}
          </Badge>
          <Badge variant="secondary" className="capitalize">
            {workspace.scope}
          </Badge>
        </div>

        <ConfigurationItem label="Name" description="Display name for this workspace">
          <ConfigurationInput
            value={formData.name || ''}
            onChange={(e) => updateField('name', e.target.value)}
            disabled={isGitSynced}
          />
        </ConfigurationItem>

        <ConfigurationItem label="Description" description="Optional description of this workspace">
          <ConfigurationTextarea
            value={formData.description || ''}
            onChange={(e) => updateField('description', e.target.value)}
            rows={3}
            disabled={isGitSynced}
          />
        </ConfigurationItem>
      </ConfigurationGroup>

      <WorkspaceReposPanel
        workspaceId={workspaceId}
        placements={workspace.repos}
        readOnly={isGitSynced}
      />

      <CodeExecutionGroup formData={formData} onUpdateField={updateField} disabled={isGitSynced} />

      <ServersGroup formData={formData} onUpdateField={updateField} disabled={isGitSynced} />

      <AgentConfigurationGroup
        formData={formData}
        onUpdateField={updateField}
        disabled={isGitSynced}
      />

      <PluginsGroup formData={formData} onUpdateField={updateField} disabled={isGitSynced} />

      {!isGitSynced && (
        <div className="flex justify-between gap-3 border-t pt-6">
          <Button
            type="button"
            variant="destructive"
            onClick={() => setShowDeleteDialog(true)}
            disabled={deleteWorkspace.isPending}
          >
            {deleteWorkspace.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Deleting...
              </>
            ) : (
              <>
                <Trash2 className="mr-2 h-4 w-4" />
                Delete Workspace
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
              disabled={!hasChanges || updateWorkspace.isPending}
            >
              {updateWorkspace.isPending ? (
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
      )}

      <AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Workspace</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{workspace.name}"? This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteWorkspace.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDelete}
              disabled={deleteWorkspace.isPending}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteWorkspace.isPending ? 'Deleting...' : 'Delete'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ConfigurationSection>
  )
}
