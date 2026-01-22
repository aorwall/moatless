import type { UpdateRepositoryRequest } from '@moatless/types'
import { ArrowRight, FileCode, Loader2, Save, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useConvertToTemplate,
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
import { ChannelsGroup } from './channels-group'
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
  const convertToTemplate = useConvertToTemplate()
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const [showConvertDialog, setShowConvertDialog] = useState(false)

  // Derive base form data from repository - following derived state pattern
  const baseFormData = useMemo<Partial<UpdateRepositoryRequest>>(() => {
    if (!repository) {
      return {}
    }

    const base: Partial<UpdateRepositoryRequest> = {
      name: repository.name || '',
      description: repository.description || '',
      systemPrompt: repository.systemPrompt || '',
      visibility: repository.visibility || 'private',
      codeExecution: repository.codeExecution || {},
      previewConfig: {
        enableInspector: true,
        ...repository.previewConfig,
      },
    }

    // Add remote URL and provider (both remote and template repositories use remote.url)
    base.remoteUrl = repository.remote.url || ''
    base.provider = repository.remote.provider

    return base
  }, [repository])

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

  const handleSave = () => {
    if (Object.keys(userModifications).length === 0) {
      return
    }

    // Send complete repository config
    // Empty strings are converted to null to signal field removal
    const payload: Partial<UpdateRepositoryRequest> = {
      ...formData,
      description: formData.description?.trim() || null,
      remoteUrl: formData.remoteUrl?.trim() || null,
      systemPrompt: formData.systemPrompt?.trim() || null,
    }

    updateRepository.mutate(
      { repoId: repositoryId, request: payload },
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

  const handleConvert = () => {
    convertToTemplate.mutate(repositoryId, {
      onSuccess: () => {
        toast.success('Repository converted to template successfully')
        setShowConvertDialog(false)
      },
      onError: (error) => {
        toast.error('Failed to convert repository', {
          description: getErrorMessage(error),
        })
        setShowConvertDialog(false)
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
      <GeneralSettingsGroup
        repositoryKind={repository.kind}
        formData={formData}
        onUpdateField={updateField}
      />

      <ChannelsGroup channels={repository.channels} repositoryId={repositoryId} />

      <CodeExecutionGroup formData={formData} onUpdateField={updateField} />

      <PreviewEnvironmentGroup formData={formData} onUpdateField={updateField} />

      <AgentConfigurationGroup formData={formData} onUpdateField={updateField} />
      <div className="flex justify-between gap-3 pt-6 border-t">
        <div className="flex gap-3">
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
          {repository.kind === 'remote' && (
            <Button
              type="button"
              variant="outline"
              onClick={() => setShowConvertDialog(true)}
              disabled={convertToTemplate.isPending}
            >
              {convertToTemplate.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Converting...
                </>
              ) : (
                <>
                  <FileCode className="mr-2 h-4 w-4" />
                  Convert to Template
                </>
              )}
            </Button>
          )}
        </div>
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
      <AlertDialog open={showConvertDialog} onOpenChange={setShowConvertDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Convert to Template Repository</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to convert "{repository?.name}" to a template repository?
              <div className="mt-3 space-y-2 text-sm">
                <div className="flex items-center gap-2">
                  <ArrowRight className="h-4 w-4 text-muted-foreground" />
                  <span>Template repositories cannot be used directly with tasks</span>
                </div>
                <div className="flex items-center gap-2">
                  <ArrowRight className="h-4 w-4 text-muted-foreground" />
                  <span>They serve as reusable starting points for creating new repositories</span>
                </div>
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={convertToTemplate.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleConvert} disabled={convertToTemplate.isPending}>
              {convertToTemplate.isPending ? 'Converting...' : 'Convert to Template'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </ConfigurationSection>
  )
}
