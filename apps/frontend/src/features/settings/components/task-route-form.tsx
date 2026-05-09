import type { RoutingMode, TaskRouteResponse } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useCreateTaskRoute, useUpdateTaskRoute } from '@/features/settings/hooks/use-integrations'
import { useTagsList } from '@/features/tags'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

interface TaskRouteFormProps {
  route?: TaskRouteResponse | null
  onSuccess?: () => void
  onCancel?: () => void
}

interface TaskRouteFormState {
  repositoryId: string
  routingMode: RoutingMode
  promptTemplate: string
  taskName: string
  tagIds: string[]
}

const ROUTING_MODE_DESCRIPTIONS: Record<RoutingMode, string> = {
  by_subject:
    'Each external subject (Slack thread, GitHub PR, Linear issue) gets one task. Replies on that subject reuse the task.',
  ongoing: 'All messages route to a single long-lived task.',
}

function makeInitialForm(route: TaskRouteResponse | null | undefined): TaskRouteFormState {
  return {
    repositoryId: route?.repositoryId ?? '',
    routingMode: route?.routingMode ?? 'by_subject',
    promptTemplate: route?.promptTemplate ?? '',
    taskName: route?.taskName ?? '',
    tagIds: route?.tagIds ?? [],
  }
}

export function TaskRouteForm({ route, onSuccess, onCancel }: TaskRouteFormProps) {
  const { data: repositories } = useRepositoriesList()
  const { data: tags } = useTagsList()
  const repositoryItems = repositories ?? []
  const tagItems = tags ?? []
  const nonTemplateRepos = repositoryItems.filter((repo) => repo.kind !== 'template')

  const [form, setForm] = useState<TaskRouteFormState>(() => makeInitialForm(route))
  const createRoute = useCreateTaskRoute()
  const updateRoute = useUpdateTaskRoute()
  const isEditing = !!route
  const isPending = createRoute.isPending || updateRoute.isPending

  useEffect(() => {
    setForm(makeInitialForm(route))
  }, [route])

  const handleSubmit = () => {
    if (!form.repositoryId) {
      toast.error('Select a repository')
      return
    }

    const request = {
      repositoryId: form.repositoryId,
      routingMode: form.routingMode,
      promptTemplate: form.promptTemplate.trim() || null,
      taskName: form.taskName.trim() || null,
      tagIds: form.tagIds,
    }

    if (isEditing && route) {
      updateRoute.mutate(
        { id: route.id, request },
        {
          onSuccess: () => {
            toast.success('Task route updated')
            onSuccess?.()
          },
          onError: (error) => {
            toast.error('Failed to update task route', {
              description: getErrorMessage(error),
            })
          },
        },
      )
      return
    }

    createRoute.mutate(request, {
      onSuccess: () => {
        toast.success('Task route created')
        onSuccess?.()
      },
      onError: (error) => {
        toast.error('Failed to create task route', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label>Repository</Label>
        <Select
          value={form.repositoryId}
          onValueChange={(repositoryId) => setForm((prev) => ({ ...prev, repositoryId }))}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select a repository" />
          </SelectTrigger>
          <SelectContent>
            {nonTemplateRepos.map((repo) => (
              <SelectItem key={repo.id} value={repo.id}>
                {repo.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-2">
        <Label>Routing Mode</Label>
        <Select
          value={form.routingMode}
          onValueChange={(routingMode) =>
            setForm((prev) => ({ ...prev, routingMode: routingMode as RoutingMode }))
          }
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="by_subject">By Subject</SelectItem>
            <SelectItem value="ongoing">Ongoing</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          {ROUTING_MODE_DESCRIPTIONS[form.routingMode]}
        </p>
      </div>

      <div className="space-y-2">
        <Label>
          Prompt Template <span className="text-muted-foreground">(optional)</span>
        </Label>
        <Textarea
          value={form.promptTemplate}
          onChange={(event) => setForm((prev) => ({ ...prev, promptTemplate: event.target.value }))}
          placeholder="Custom prompt wrapping the source message. Use {message} as placeholder."
          rows={3}
        />
      </div>

      <div className="space-y-2">
        <Label>
          Task Name <span className="text-muted-foreground">(optional)</span>
        </Label>
        <Input
          value={form.taskName}
          onChange={(event) => setForm((prev) => ({ ...prev, taskName: event.target.value }))}
          placeholder="Default task name"
        />
      </div>

      {tagItems.length > 0 && (
        <div className="space-y-2">
          <Label>
            Tags <span className="text-muted-foreground">(optional)</span>
          </Label>
          <div className="flex flex-wrap gap-2">
            {tagItems.map((tag) => {
              const selected = form.tagIds.includes(tag.tagId)
              return (
                <button
                  key={tag.tagId}
                  type="button"
                  className={`inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold transition-colors ${
                    selected
                      ? 'border-primary bg-primary text-primary-foreground'
                      : 'border-border bg-background text-foreground hover:bg-accent'
                  }`}
                  onClick={() =>
                    setForm((prev) => ({
                      ...prev,
                      tagIds: selected
                        ? prev.tagIds.filter((id) => id !== tag.tagId)
                        : [...prev.tagIds, tag.tagId],
                    }))
                  }
                >
                  {tag.name}
                </button>
              )
            })}
          </div>
        </div>
      )}

      <div className="flex justify-end gap-2 pt-2">
        {onCancel && (
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
        )}
        <Button onClick={handleSubmit} disabled={isPending || !form.repositoryId}>
          {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
          {isEditing ? 'Save Changes' : 'Create'}
        </Button>
      </div>
    </div>
  )
}
