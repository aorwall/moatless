import type { PermissionMode, UpdateClaudeCodeSettingsRequest } from '@moatless/types'
import { AlertCircle, Loader2, RotateCcw, Save } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useClaudeCodeSettings,
  useClaudeCodeTools,
  useDeleteClaudeCodeSettings,
  useUpdateClaudeCodeSettings,
} from '@/features/claude-code'
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
import { Checkbox } from '@/lib/components/ui/checkbox'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Separator } from '@/lib/components/ui/separator'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

export function ClaudeCodeConfigurationSection() {
  const { data: settings, isLoading, error } = useClaudeCodeSettings()
  const { data: toolsData } = useClaudeCodeTools()
  const updateSettings = useUpdateClaudeCodeSettings()
  const deleteSettings = useDeleteClaudeCodeSettings()

  const modelId = useId()
  const maxTurnsId = useId()
  const maxThinkingTokensId = useId()
  const systemPromptId = useId()
  const appendSystemPromptId = useId()
  const permissionModeId = useId()

  const [showBypassWarning, setShowBypassWarning] = useState(false)

  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const baseClaudeFormData = useMemo<UpdateClaudeCodeSettingsRequest>(
    () => ({
      model: settings?.model || null,
      maxTurns: settings?.maxTurns || null,
      maxThinkingTokens: settings?.maxThinkingTokens || 8000,
      systemPrompt: settings?.systemPrompt || null,
      appendSystemPrompt: settings?.appendSystemPrompt || null,
      allowedTools: settings?.allowedTools || [],
      disallowedTools: settings?.disallowedTools || [],
      permissionMode: settings?.permissionMode || null,
    }),
    [settings],
  )

  // Track user modifications separately from the derived data
  const [claudeUserModifications, setClaudeUserModifications] = useState<
    Partial<UpdateClaudeCodeSettingsRequest>
  >({})

  // Merge derived data with user modifications
  const formData = useMemo(
    () => ({
      ...baseClaudeFormData,
      ...claudeUserModifications,
    }),
    [baseClaudeFormData, claudeUserModifications],
  )

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    if (formData.permissionMode === 'bypassPermissions' && !showBypassWarning) {
      setShowBypassWarning(true)
      return
    }

    updateSettings.mutate(formData, {
      onSuccess: () => {
        // Clear user modifications since settings are now persisted
        setClaudeUserModifications({})
        toast.success('Settings saved successfully')
      },
      onError: (error) => {
        toast.error('Failed to save settings', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleReset = () => {
    deleteSettings.mutate(undefined, {
      onSuccess: () => {
        // Clear user modifications when reset succeeds
        setClaudeUserModifications({})
        toast.success('Settings reset to defaults')
      },
      onError: (error) => {
        toast.error('Failed to reset settings', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleToolToggle = (toolName: string, listType: 'allowed' | 'disallowed') => {
    const otherList = listType === 'allowed' ? 'disallowedTools' : 'allowedTools'
    const currentList = listType === 'allowed' ? 'allowedTools' : 'disallowedTools'

    setClaudeUserModifications((prev) => {
      // Get current values from merged formData to ensure we're working with the latest state
      const currentOtherList = formData[otherList] || []
      const currentCurrentList = formData[currentList] || []

      // Remove from other list if present
      const newOtherList = currentOtherList.filter((t) => t !== toolName)

      // Toggle in current list
      const newCurrentList = currentCurrentList.includes(toolName)
        ? currentCurrentList.filter((t) => t !== toolName)
        : [...currentCurrentList, toolName]

      return {
        ...prev,
        [otherList]: newOtherList,
        [currentList]: newCurrentList,
      }
    })
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-96">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertCircle className="h-4 w-4" />
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>
          Failed to load Claude Code settings. Please try again later.
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <div className="space-y-6">
      <form onSubmit={handleSubmit} className="space-y-6">
        {/* Model Configuration */}
        <div className="space-y-6">
          <div className="space-y-1">
            <h3 className="text-base font-medium">Model Configuration</h3>
            <p className="text-sm text-muted-foreground">
              Configure the AI model and conversation settings
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={modelId} className="text-sm font-medium">
                Model
              </Label>
              <p className="text-sm text-muted-foreground">Override the default model (optional)</p>
            </div>
            <Input
              id={modelId}
              type="text"
              placeholder="e.g., claude-3-opus-20240229"
              value={formData.model || ''}
              onChange={(e) =>
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  model: e.target.value || null,
                }))
              }
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={maxTurnsId} className="text-sm font-medium">
                Max Turns
              </Label>
              <p className="text-sm text-muted-foreground">Maximum number of conversation turns</p>
            </div>
            <Input
              id={maxTurnsId}
              type="number"
              min="1"
              max="100"
              placeholder="Default: unlimited"
              value={formData.maxTurns || ''}
              onChange={(e) =>
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  maxTurns: e.target.value ? Number.parseInt(e.target.value) : null,
                }))
              }
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={maxThinkingTokensId} className="text-sm font-medium">
                Max Thinking Tokens
              </Label>
              <p className="text-sm text-muted-foreground">Token limit for model reasoning</p>
            </div>
            <Input
              id={maxThinkingTokensId}
              type="number"
              min="0"
              max="100000"
              value={formData.maxThinkingTokens}
              onChange={(e) =>
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  maxThinkingTokens: Number.parseInt(e.target.value) || 8000,
                }))
              }
            />
          </div>
        </div>

        <Separator />

        {/* System Prompts */}
        <div className="space-y-6">
          <div className="space-y-1">
            <h3 className="text-base font-medium">System Prompts</h3>
            <p className="text-sm text-muted-foreground">
              Customize the agent's behavior with system prompts
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={systemPromptId} className="text-sm font-medium">
                System Prompt
              </Label>
              <p className="text-sm text-muted-foreground">Override the default system prompt</p>
            </div>
            <Textarea
              id={systemPromptId}
              placeholder="Override the default system prompt"
              rows={4}
              value={formData.systemPrompt || ''}
              onChange={(e) =>
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  systemPrompt: e.target.value || null,
                }))
              }
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={appendSystemPromptId} className="text-sm font-medium">
                Append System Prompt
              </Label>
              <p className="text-sm text-muted-foreground">
                Additional instructions to append to the system prompt
              </p>
            </div>
            <Textarea
              id={appendSystemPromptId}
              placeholder="Additional instructions to append to the system prompt"
              rows={4}
              value={formData.appendSystemPrompt || ''}
              onChange={(e) =>
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  appendSystemPrompt: e.target.value || null,
                }))
              }
            />
          </div>
        </div>

        <Separator />

        {/* Permission Mode */}
        <div className="space-y-6">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <h3 className="text-base font-medium">Permission Mode</h3>
              <Badge variant="destructive" className="text-xs">
                Security
              </Badge>
            </div>
            <p className="text-sm text-muted-foreground">
              Control how Claude Code handles tool permissions
            </p>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-start">
            <div className="space-y-1">
              <Label htmlFor={permissionModeId} className="text-sm font-medium">
                Mode
              </Label>
              <p className="text-sm text-muted-foreground">Select permission handling mode</p>
            </div>
            <Select
              value={formData.permissionMode || 'default'}
              onValueChange={(value) =>
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  permissionMode: value === 'default' ? null : (value as PermissionMode),
                }))
              }
            >
              <SelectTrigger id={permissionModeId}>
                <SelectValue placeholder="Select permission mode" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="default">Default</SelectItem>
                <SelectItem value="acceptEdits">Accept Edits</SelectItem>
                <SelectItem value="bypassPermissions">Bypass Permissions ⚠️</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {formData.permissionMode === 'bypassPermissions' && (
            <Alert variant="destructive" className="md:col-start-2">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription>
                This mode allows Claude Code to perform any action without asking for permission.
              </AlertDescription>
            </Alert>
          )}
        </div>
      </form>

      <Separator />

      {/* Tool Configuration */}
      {toolsData?.tools && (
        <div className="space-y-6">
          <div className="space-y-1">
            <h3 className="text-base font-medium">Tool Configuration</h3>
            <p className="text-sm text-muted-foreground">
              Configure which tools Claude Code can use
            </p>
          </div>

          <div className="space-y-4">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <span>Tools marked with 🔒 require permissions</span>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
              {toolsData.tools.map((tool) => (
                <div
                  key={tool.name}
                  className="space-y-3 p-4 border rounded-lg hover:bg-muted/50 transition-colors"
                >
                  <div className="flex items-start justify-between">
                    <div className="flex-1 min-w-0">
                      <div className="font-medium flex items-center gap-2">
                        <span className="truncate">{tool.name}</span>
                        {tool.permissionRequired && (
                          <Badge variant="outline" className="text-xs px-1 py-0">
                            🔒
                          </Badge>
                        )}
                      </div>
                      <p className="text-sm text-muted-foreground mt-1 line-clamp-2">
                        {tool.description}
                      </p>
                    </div>
                  </div>
                  <div className="flex items-center gap-4">
                    <div className="flex items-center space-x-2">
                      <Checkbox
                        id={`${tool.name}-allowed`}
                        data-testid={`${tool.name}-allowed`}
                        checked={formData.allowedTools?.includes(tool.name)}
                        onCheckedChange={() => handleToolToggle(tool.name, 'allowed')}
                      />
                      <Label
                        htmlFor={`${tool.name}-allowed`}
                        className="text-sm font-normal cursor-pointer text-green-600"
                      >
                        Allow
                      </Label>
                    </div>
                    <div className="flex items-center space-x-2">
                      <Checkbox
                        id={`${tool.name}-disallowed`}
                        data-testid={`${tool.name}-disallowed`}
                        checked={formData.disallowedTools?.includes(tool.name)}
                        onCheckedChange={() => handleToolToggle(tool.name, 'disallowed')}
                      />
                      <Label
                        htmlFor={`${tool.name}-disallowed`}
                        className="text-sm font-normal cursor-pointer text-red-600"
                      >
                        Disallow
                      </Label>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      <Separator />

      {/* Claude Code Actions */}
      <div className="flex flex-col gap-3 sm:flex-row sm:justify-between pt-4">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={handleReset}
          disabled={deleteSettings.isPending}
        >
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset Claude Code Settings
        </Button>
        <Button type="submit" disabled={updateSettings.isPending} size="sm" onClick={handleSubmit}>
          {updateSettings.isPending ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Save className="mr-2 h-4 w-4" />
          )}
          Save Claude Code Settings
        </Button>
      </div>

      {/* Bypass Permissions Warning Dialog */}
      <AlertDialog open={showBypassWarning} onOpenChange={setShowBypassWarning}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>⚠️ Bypass Permissions Warning</AlertDialogTitle>
            <AlertDialogDescription className="space-y-2">
              <p>You are about to enable "Bypass Permissions" mode. This means:</p>
              <ul className="list-disc pl-5 space-y-1">
                <li>Claude Code will be able to execute ANY command without asking</li>
                <li>Files can be created, modified, or deleted without confirmation</li>
                <li>System commands can be run without your approval</li>
                <li>This could potentially be dangerous if misused</li>
              </ul>
              <p className="font-semibold">
                Only enable this if you fully trust the code being executed and understand the
                risks.
              </p>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                setClaudeUserModifications((prev) => ({
                  ...prev,
                  permissionMode: null,
                }))
              }}
            >
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setShowBypassWarning(false)
                // Create a synthetic form event
                const syntheticEvent = {
                  preventDefault: () => {
                    /* no-op - synthetic event */
                  },
                } as React.FormEvent
                handleSubmit(syntheticEvent)
              }}
              className="bg-amber-600 hover:bg-amber-700"
            >
              I Understand the Risks
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
