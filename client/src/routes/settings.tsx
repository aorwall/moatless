import type { PermissionMode, UpdateClaudeCodeSettingsRequest } from '@shared/types/claude-code'
import { createFileRoute } from '@tanstack/react-router'
import { AlertCircle, Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
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
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
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
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/settings')({
  component: SettingsPage,
})

function SettingsPage() {
  const { data: settings, isLoading, error } = useClaudeCodeSettings()
  const { data: toolsData } = useClaudeCodeTools()
  const updateSettings = useUpdateClaudeCodeSettings()
  const deleteSettings = useDeleteClaudeCodeSettings()

  const [showBypassWarning, setShowBypassWarning] = useState(false)
  const [formData, setFormData] = useState<UpdateClaudeCodeSettingsRequest>({
    model: null,
    maxTurns: null,
    maxThinkingTokens: 8000,
    systemPrompt: null,
    appendSystemPrompt: null,
    allowedTools: [],
    disallowedTools: [],
    permissionMode: null,
  })

  // Update form data when settings are loaded
  useEffect(() => {
    if (settings) {
      setFormData({
        model: settings.model,
        maxTurns: settings.maxTurns,
        maxThinkingTokens: settings.maxThinkingTokens,
        systemPrompt: settings.systemPrompt,
        appendSystemPrompt: settings.appendSystemPrompt,
        allowedTools: settings.allowedTools,
        disallowedTools: settings.disallowedTools,
        permissionMode: settings.permissionMode,
      })
    }
  }, [settings])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    if (formData.permissionMode === 'bypassPermissions' && !showBypassWarning) {
      setShowBypassWarning(true)
      return
    }

    updateSettings.mutate(formData, {
      onSuccess: () => {
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
        // Reset form to defaults
        setFormData({
          model: null,
          maxTurns: null,
          maxThinkingTokens: 8000,
          systemPrompt: null,
          appendSystemPrompt: null,
          allowedTools: [],
          disallowedTools: [],
          permissionMode: null,
        })
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

    setFormData((prev) => {
      // Remove from other list if present
      const newOtherList = prev[otherList]?.filter((t) => t !== toolName) || []

      // Toggle in current list
      const currentTools = prev[currentList] || []
      const newCurrentList = currentTools.includes(toolName)
        ? currentTools.filter((t) => t !== toolName)
        : [...currentTools, toolName]

      return {
        ...prev,
        [otherList]: newOtherList,
        [currentList]: newCurrentList,
      }
    })
  }

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">Settings</h1>
          <p className="text-muted-foreground mt-2">
            Manage your application settings and configurations
          </p>
        </div>

        <div className="space-y-6">
          {/* Claude Code Settings */}
          {isLoading ? (
            <div className="flex items-center justify-center h-96">
              <Loader2 className="h-8 w-8 animate-spin" />
            </div>
          ) : error ? (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertTitle>Error</AlertTitle>
              <AlertDescription>
                Failed to load Claude Code settings. Please try again later.
              </AlertDescription>
            </Alert>
          ) : (
            <form onSubmit={handleSubmit} className="space-y-6">
              {/* Model Configuration */}
              <Card>
                <CardHeader>
                  <CardTitle>Model Configuration</CardTitle>
                  <CardDescription>
                    Configure the AI model and conversation settings
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="space-y-2">
                    <Label htmlFor="model">Model</Label>
                    <Input
                      id="model"
                      type="text"
                      placeholder="e.g., claude-3-opus-20240229"
                      value={formData.model || ''}
                      onChange={(e) =>
                        setFormData({
                          ...formData,
                          model: e.target.value || null,
                        })
                      }
                    />
                    <p className="text-sm text-muted-foreground">
                      Override the default model (optional)
                    </p>
                  </div>

                  <div className="grid grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <Label htmlFor="max_turns">Max Turns</Label>
                      <Input
                        id="max_turns"
                        type="number"
                        min="1"
                        max="100"
                        placeholder="Default: unlimited"
                        value={formData.maxTurns || ''}
                        onChange={(e) =>
                          setFormData({
                            ...formData,
                            maxTurns: e.target.value ? parseInt(e.target.value) : null,
                          })
                        }
                      />
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="max_thinking_tokens">Max Thinking Tokens</Label>
                      <Input
                        id="max_thinking_tokens"
                        type="number"
                        min="0"
                        max="100000"
                        value={formData.maxThinkingTokens}
                        onChange={(e) =>
                          setFormData({
                            ...formData,
                            maxThinkingTokens: parseInt(e.target.value) || 8000,
                          })
                        }
                      />
                    </div>
                  </div>
                </CardContent>
              </Card>

              {/* System Prompts */}
              <Card>
                <CardHeader>
                  <CardTitle>System Prompts</CardTitle>
                  <CardDescription>
                    Customize the agent's behavior with system prompts
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="space-y-2">
                    <Label htmlFor="system_prompt">System Prompt</Label>
                    <Textarea
                      id="system_prompt"
                      placeholder="Override the default system prompt"
                      rows={4}
                      value={formData.systemPrompt || ''}
                      onChange={(e) =>
                        setFormData({
                          ...formData,
                          systemPrompt: e.target.value || null,
                        })
                      }
                    />
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="append_system_prompt">Append System Prompt</Label>
                    <Textarea
                      id="append_system_prompt"
                      placeholder="Additional instructions to append to the system prompt"
                      rows={4}
                      value={formData.appendSystemPrompt || ''}
                      onChange={(e) =>
                        setFormData({
                          ...formData,
                          appendSystemPrompt: e.target.value || null,
                        })
                      }
                    />
                  </div>
                </CardContent>
              </Card>

              {/* Permission Mode */}
              <Card>
                <CardHeader>
                  <CardTitle>Permission Mode</CardTitle>
                  <CardDescription>
                    Control how Claude Code handles tool permissions
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="space-y-2">
                    <Label htmlFor="permission_mode">Mode</Label>
                    <Select
                      value={formData.permissionMode || 'default'}
                      onValueChange={(value) =>
                        setFormData({
                          ...formData,
                          permissionMode: value === 'default' ? null : (value as PermissionMode),
                        })
                      }
                    >
                      <SelectTrigger id="permission_mode">
                        <SelectValue placeholder="Select permission mode" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="default">Default</SelectItem>
                        <SelectItem value="acceptEdits">Accept Edits</SelectItem>
                        <SelectItem value="bypassPermissions">Bypass Permissions ⚠️</SelectItem>
                      </SelectContent>
                    </Select>
                    <p className="text-sm text-muted-foreground">
                      {formData.permissionMode === 'bypassPermissions' && (
                        <span className="text-amber-600">
                          Warning: This mode allows Claude Code to perform any action without asking
                          for permission.
                        </span>
                      )}
                    </p>
                  </div>
                </CardContent>
              </Card>

              {/* Tool Configuration */}
              <Card>
                <CardHeader>
                  <CardTitle>Tool Configuration</CardTitle>
                  <CardDescription>Configure which tools Claude Code can use</CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  {toolsData?.tools && (
                    <div className="space-y-4">
                      <div className="text-sm text-muted-foreground mb-2">
                        Tools marked with 🔒 require permissions
                      </div>
                      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                        {toolsData.tools.map((tool) => (
                          <div key={tool.name} className="space-y-2 p-3 border rounded-lg">
                            <div className="flex items-start justify-between">
                              <div className="flex-1">
                                <div className="font-medium flex items-center gap-2">
                                  {tool.name}
                                  {tool.permission_required && (
                                    <span title="Requires permission">🔒</span>
                                  )}
                                </div>
                                <p className="text-sm text-muted-foreground">{tool.description}</p>
                              </div>
                            </div>
                            <div className="flex items-center gap-4">
                              <div className="flex items-center space-x-2">
                                <Checkbox
                                  id={`${tool.name}-allowed`}
                                  checked={formData.allowedTools?.includes(tool.name)}
                                  onCheckedChange={() => handleToolToggle(tool.name, 'allowed')}
                                />
                                <Label
                                  htmlFor={`${tool.name}-allowed`}
                                  className="text-sm font-normal cursor-pointer"
                                >
                                  Allow
                                </Label>
                              </div>
                              <div className="flex items-center space-x-2">
                                <Checkbox
                                  id={`${tool.name}-disallowed`}
                                  checked={formData.disallowed_tools?.includes(tool.name)}
                                  onCheckedChange={() => handleToolToggle(tool.name, 'disallowed')}
                                />
                                <Label
                                  htmlFor={`${tool.name}-disallowed`}
                                  className="text-sm font-normal cursor-pointer"
                                >
                                  Disallow
                                </Label>
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </CardContent>
              </Card>

              {/* Actions */}
              <div className="flex justify-between">
                <Button
                  type="button"
                  variant="outline"
                  onClick={handleReset}
                  disabled={deleteSettings.isPending}
                >
                  Reset to Defaults
                </Button>
                <Button type="submit" disabled={updateSettings.isPending}>
                  {updateSettings.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                  Save Settings
                </Button>
              </div>
            </form>
          )}
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
                  setFormData({ ...formData, permission_mode: null })
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
    </div>
  )
}
