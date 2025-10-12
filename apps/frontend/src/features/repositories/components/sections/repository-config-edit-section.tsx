import type { UpdateRepositoryRequest } from '@moatless/types'
import { Loader2, Save, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useDeleteRepository,
  useRepository,
  useUpdateRepository,
} from '@/features/repositories/hooks/use-repositories'
import { ConfigurationSectionWrapper as ConfigurationSection } from '@/lib/components/configuration'
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
import { getErrorMessage } from '@/lib/utils/error'
import { AgentConfigurationGroup } from './agent-configuration-group'
import { CodeExecutionGroup } from './code-execution-group'
import { GeneralSettingsGroup } from './general-settings-group'
import { PreviewEnvironmentGroup } from './preview-environment-group'

interface RepositoryConfigEditSectionProps {
  repositoryId: string
  onDeleted?: () => void
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

  // Derive base form data from repository - following derived state pattern
  const baseFormData = useMemo<Partial<UpdateRepositoryRequest>>(
    () => ({
      name: repository?.name || '',
      description: repository?.description || '',
      path: repository?.path || '',
      remoteUrl: repository?.remoteUrl || '',
      useGitWorktrees: repository?.useGitWorktrees,
      cloneOnNewTask: repository?.cloneOnNewTask,
      syncWithRepoConfig: repository?.syncWithRepoConfig,
      systemPrompt: repository?.systemPrompt || '',
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

  // When both sync and clone are enabled, config must be updated in the repository file
  const isConfigFileOnly = formData.syncWithRepoConfig && formData.cloneOnNewTask

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

  const handleSave = () => {
    if (Object.keys(userModifications).length === 0) {
      return
    }

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
      <GeneralSettingsGroup formData={formData} onUpdateField={updateField} />

      <CodeExecutionGroup
        formData={formData}
        onUpdateField={updateField}
        isConfigFileOnly={isConfigFileOnly}
      />

      <PreviewEnvironmentGroup
        formData={formData}
        onUpdateField={updateField}
        isConfigFileOnly={isConfigFileOnly}
      />

      <AgentConfigurationGroup
        formData={formData}
        onUpdateField={updateField}
        isConfigFileOnly={isConfigFileOnly}
      />
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
    </ConfigurationSection>
  )
}
