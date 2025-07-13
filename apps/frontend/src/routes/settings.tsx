import type {
  PermissionMode,
  UpdateClaudeCodeSettingsRequest,
  UpdateSettingsRequest,
} from '@moatless/shared'
import { createFileRoute } from '@tanstack/react-router'
import { AlertCircle, FolderOpen, Loader2, RotateCcw, Save } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useClaudeCodeSettings,
  useClaudeCodeTools,
  useDeleteClaudeCodeSettings,
  useUpdateClaudeCodeSettings,
} from '@/features/claude-code'
import { useResetSettings, useSettings, useUpdateSettings } from '@/features/settings'
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
import { Separator } from '@/lib/components/ui/separator'
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

  // Application settings
  const {
    data: appSettings,
    isLoading: appSettingsLoading,
    error: appSettingsError,
  } = useSettings()
  const updateAppSettings = useUpdateSettings()
  const resetAppSettings = useResetSettings()

  const modelId = useId()
  const maxTurnsId = useId()
  const maxThinkingTokensId = useId()
  const systemPromptId = useId()
  const appendSystemPromptId = useId()
  const permissionModeId = useId()
  const dataDirId = useId()
  const defaultWorkspaceTypeId = useId()
  const claudeBinDirId = useId()
  const claudeConfigDirId = useId()

  const [showBypassWarning, setShowBypassWarning] = useState(false)
  // Derive form data directly from settings - no effects needed (following derived state pattern)
  const baseAppFormData = useMemo<UpdateSettingsRequest>(
    () => ({
      dataDir: appSettings?.settings.dataDir || null,
      defaultWorkspaceType: appSettings?.settings.defaultWorkspaceType || null,
      claude: {
        binDir: appSettings?.settings.claude?.binDir || null,
        configDir: appSettings?.settings.claude?.configDir || null,
      },
    }),
    [appSettings],
  )

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
  const [appUserModifications, setAppUserModifications] = useState<Partial<UpdateSettingsRequest>>(
    {},
  )
  const [claudeUserModifications, setClaudeUserModifications] = useState<
    Partial<UpdateClaudeCodeSettingsRequest>
  >({})

  // Merge derived data with user modifications
  const appFormData = useMemo(
    () => ({
      ...baseAppFormData,
      ...appUserModifications,
      claude: {
        ...baseAppFormData.claude,
        ...appUserModifications.claude,
      },
    }),
    [baseAppFormData, appUserModifications],
  )

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

  return (
    <div className="flex min-h-screen flex-col">
      <div className="flex-1 space-y-6 p-6">
        <div className="mx-auto max-w-6xl">
          <div className="grid gap-6 lg:grid-cols-2">
            {/* Application Settings */}
            <div className="space-y-6">
              {appSettingsLoading ? (
                <Card>
                  <CardContent className="flex items-center justify-center h-32">
                    <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
                  </CardContent>
                </Card>
              ) : appSettingsError ? (
                <Alert variant="destructive">
                  <AlertCircle className="h-4 w-4" />
                  <AlertTitle>Error</AlertTitle>
                  <AlertDescription>
                    Failed to load application settings. Please try again later.
                  </AlertDescription>
                </Alert>
              ) : (
                <Card>
                  <CardHeader>
                    <div className="flex items-center gap-2">
                      <CardTitle>Application Settings</CardTitle>
                      <Badge variant="outline">Core</Badge>
                    </div>
                    <CardDescription>Configure core application settings</CardDescription>
                  </CardHeader>
                  <CardContent className="space-y-6">
                    <div className="space-y-2">
                      <Label htmlFor={dataDirId}>Data Directory</Label>
                      <div className="flex gap-2">
                        <Input
                          id={dataDirId}
                          type="text"
                          placeholder={appSettings?.defaults.dataDir || '~/.moatless'}
                          value={appFormData.dataDir || ''}
                          onChange={(e) => {
                            const value = e.target.value.trim()
                            setAppUserModifications((prev) => ({
                              ...prev,
                              dataDir:
                                value === '' || value === appSettings?.defaults.dataDir
                                  ? null
                                  : value,
                            }))
                          }}
                        />
                        <Button
                          type="button"
                          variant="outline"
                          size="icon"
                          title="Browse folders"
                          disabled
                        >
                          <FolderOpen className="h-4 w-4" />
                        </Button>
                      </div>
                      <p className="text-sm text-muted-foreground">
                        Directory where application data is stored
                      </p>
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor={defaultWorkspaceTypeId}>Default Workspace Type</Label>
                      <Select
                        value={
                          appFormData.defaultWorkspaceType ||
                          appSettings?.defaults.defaultWorkspaceType ||
                          'local'
                        }
                        onValueChange={(value) =>
                          setAppUserModifications((prev) => ({
                            ...prev,
                            defaultWorkspaceType:
                              value === appSettings?.defaults.defaultWorkspaceType
                                ? null
                                : (value as 'docker' | 'local'),
                          }))
                        }
                      >
                        <SelectTrigger id={defaultWorkspaceTypeId}>
                          <SelectValue placeholder="Select workspace type" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="local">Local</SelectItem>
                          <SelectItem value="docker">Docker</SelectItem>
                        </SelectContent>
                      </Select>
                      <p className="text-sm text-muted-foreground">
                        Default workspace type for new tasks
                      </p>
                    </div>

                    <div className="grid grid-cols-2 gap-4">
                      <div className="space-y-2">
                        <Label htmlFor={claudeBinDirId}>Claude Bin Directory</Label>
                        <Input
                          id={claudeBinDirId}
                          type="text"
                          placeholder={
                            appSettings?.defaults.claude.binDir || 'Auto-detect (optional)'
                          }
                          value={appFormData.claude?.binDir || ''}
                          onChange={(e) => {
                            const value = e.target.value.trim()
                            setAppUserModifications((prev) => ({
                              ...prev,
                              claude: {
                                ...prev.claude,
                                binDir:
                                  value === '' || value === appSettings?.defaults.claude.binDir
                                    ? null
                                    : value,
                              },
                            }))
                          }}
                        />
                        <p className="text-sm text-muted-foreground">
                          Directory containing Claude Code binary (optional - auto-detected if
                          empty)
                        </p>
                      </div>

                      <div className="space-y-2">
                        <Label htmlFor={claudeConfigDirId}>Claude Config Directory</Label>
                        <Input
                          id={claudeConfigDirId}
                          type="text"
                          placeholder={
                            appSettings?.defaults.claude.configDir || 'Auto-detect (optional)'
                          }
                          value={appFormData.claude?.configDir || ''}
                          onChange={(e) => {
                            const value = e.target.value.trim()
                            setAppUserModifications((prev) => ({
                              ...prev,
                              claude: {
                                ...prev.claude,
                                configDir:
                                  value === '' || value === appSettings?.defaults.claude.configDir
                                    ? null
                                    : value,
                              },
                            }))
                          }}
                        />
                        <p className="text-sm text-muted-foreground">
                          Directory for Claude Code configuration (optional - auto-detected if
                          empty)
                        </p>
                      </div>
                    </div>

                    <Separator />
                    <div className="flex flex-col gap-3 sm:flex-row sm:justify-between">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          resetAppSettings.mutate(undefined, {
                            onSuccess: () => {
                              // Clear user modifications when reset succeeds
                              setAppUserModifications({})
                              toast.success('Application settings reset to defaults')
                            },
                            onError: (error) => {
                              toast.error('Failed to reset application settings', {
                                description: getErrorMessage(error),
                              })
                            },
                          })
                        }}
                        disabled={resetAppSettings.isPending}
                      >
                        <RotateCcw className="mr-2 h-4 w-4" />
                        Reset to Defaults
                      </Button>
                      <Button
                        onClick={() => {
                          updateAppSettings.mutate(appFormData, {
                            onSuccess: () => {
                              // Clear user modifications since settings are now persisted
                              setAppUserModifications({})
                              toast.success('Application settings saved successfully')
                            },
                            onError: (error) => {
                              toast.error('Failed to save application settings', {
                                description: getErrorMessage(error),
                              })
                            },
                          })
                        }}
                        disabled={updateAppSettings.isPending}
                        size="sm"
                      >
                        {updateAppSettings.isPending ? (
                          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                        ) : (
                          <Save className="mr-2 h-4 w-4" />
                        )}
                        Save Settings
                      </Button>
                    </div>
                  </CardContent>
                </Card>
              )}
            </div>

            {/* Claude Code Settings */}
            <div className="space-y-6">
              {isLoading ? (
                <Card>
                  <CardContent className="flex items-center justify-center h-96">
                    <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
                  </CardContent>
                </Card>
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
                      <div className="flex items-center gap-2">
                        <CardTitle>Model Configuration</CardTitle>
                        <Badge variant="secondary">Claude Code</Badge>
                      </div>
                      <CardDescription>
                        Configure the AI model and conversation settings
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="space-y-4">
                      <div className="space-y-2">
                        <Label htmlFor={modelId}>Model</Label>
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
                        <p className="text-sm text-muted-foreground">
                          Override the default model (optional)
                        </p>
                      </div>

                      <div className="grid grid-cols-2 gap-4">
                        <div className="space-y-2">
                          <Label htmlFor={maxTurnsId}>Max Turns</Label>
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
                                maxTurns: e.target.value ? parseInt(e.target.value) : null,
                              }))
                            }
                          />
                        </div>

                        <div className="space-y-2">
                          <Label htmlFor={maxThinkingTokensId}>Max Thinking Tokens</Label>
                          <Input
                            id={maxThinkingTokensId}
                            type="number"
                            min="0"
                            max="100000"
                            value={formData.maxThinkingTokens}
                            onChange={(e) =>
                              setClaudeUserModifications((prev) => ({
                                ...prev,
                                maxThinkingTokens: parseInt(e.target.value) || 8000,
                              }))
                            }
                          />
                        </div>
                      </div>
                    </CardContent>
                  </Card>

                  {/* System Prompts */}
                  <Card>
                    <CardHeader>
                      <div className="flex items-center gap-2">
                        <CardTitle>System Prompts</CardTitle>
                        <Badge variant="secondary">Claude Code</Badge>
                      </div>
                      <CardDescription>
                        Customize the agent's behavior with system prompts
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="space-y-4">
                      <div className="space-y-2">
                        <Label htmlFor={systemPromptId}>System Prompt</Label>
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

                      <div className="space-y-2">
                        <Label htmlFor={appendSystemPromptId}>Append System Prompt</Label>
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
                    </CardContent>
                  </Card>

                  {/* Permission Mode */}
                  <Card>
                    <CardHeader>
                      <div className="flex items-center gap-2">
                        <CardTitle>Permission Mode</CardTitle>
                        <Badge variant="destructive">Security</Badge>
                      </div>
                      <CardDescription>
                        Control how Claude Code handles tool permissions
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="space-y-4">
                      <div className="space-y-2">
                        <Label htmlFor={permissionModeId}>Mode</Label>
                        <Select
                          value={formData.permissionMode || 'default'}
                          onValueChange={(value) =>
                            setClaudeUserModifications((prev) => ({
                              ...prev,
                              permissionMode:
                                value === 'default' ? null : (value as PermissionMode),
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
                        <p className="text-sm text-muted-foreground">
                          {formData.permissionMode === 'bypassPermissions' && (
                            <span className="text-amber-600">
                              Warning: This mode allows Claude Code to perform any action without
                              asking for permission.
                            </span>
                          )}
                        </p>
                      </div>
                    </CardContent>
                  </Card>
                </form>
              )}
            </div>
          </div>

          {/* Tool Configuration - Full Width */}
          {!isLoading && !error && (
            <Card>
              <CardHeader>
                <div className="flex items-center gap-2">
                  <CardTitle>Tool Configuration</CardTitle>
                  <Badge variant="secondary">Claude Code</Badge>
                </div>
                <CardDescription>Configure which tools Claude Code can use</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                {toolsData?.tools && (
                  <div className="space-y-6">
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

                    <Separator />

                    {/* Claude Code Actions */}
                    <div className="flex flex-col gap-3 sm:flex-row sm:justify-between">
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
                      <Button
                        type="submit"
                        disabled={updateSettings.isPending}
                        size="sm"
                        onClick={handleSubmit}
                      >
                        {updateSettings.isPending ? (
                          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                        ) : (
                          <Save className="mr-2 h-4 w-4" />
                        )}
                        Save Claude Code Settings
                      </Button>
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>
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
                  setClaudeUserModifications((prev) => ({ ...prev, permissionMode: null }))
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
