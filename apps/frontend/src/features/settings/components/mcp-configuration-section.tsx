import type { McpServerConfig, UpdateMcpSettingsRequest } from '@moatless/types'
import { Loader2, Plus, Trash2, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useMcpSettings, useUpdateMcpSettings } from '@/features/settings'
import {
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { getErrorMessage } from '@/lib/utils/error'

interface ServerFormData {
  name: string
  type: 'stdio' | 'http'
  command?: string
  args?: string[]
  env?: Record<string, string>
  url?: string
  headers?: Record<string, string>
}

export function McpConfigurationSection() {
  const { data: mcpSettings, isLoading: mcpLoading, error: mcpError } = useMcpSettings()
  const updateMcpSettings = useUpdateMcpSettings()

  // Convert settings to form data
  const baseMcpFormData = useMemo<Record<string, McpServerConfig>>(() => {
    return mcpSettings?.settings?.mcpServers || {}
  }, [mcpSettings])

  // Track user modifications
  const [mcpUserModifications, setMcpUserModifications] = useState<Record<string, McpServerConfig>>(
    {},
  )

  // Track servers being edited
  const [editingServer, setEditingServer] = useState<string | null>(null)
  const [serverFormData, setServerFormData] = useState<ServerFormData>({
    name: '',
    type: 'stdio',
    command: 'npx',
    args: [],
    env: {},
    url: '',
    headers: {},
  })

  // Merge base data with modifications
  const mcpFormData = useMemo(
    () => ({
      ...baseMcpFormData,
      ...mcpUserModifications,
    }),
    [baseMcpFormData, mcpUserModifications],
  )

  const handleAddServer = () => {
    setEditingServer('__new__')
    setServerFormData({
      name: '',
      type: 'stdio',
      command: 'npx',
      args: [],
      env: {},
      url: '',
      headers: {},
    })
  }

  const handleEditServer = (serverName: string) => {
    const server = mcpFormData[serverName]
    if (!server) {
      return
    }

    setEditingServer(serverName)

    if ('type' in server && server.type === 'http') {
      setServerFormData({
        name: serverName,
        type: 'http',
        url: server.url,
        headers: server.headers,
      })
    } else {
      // Type narrowing: if not HTTP, it must be stdio
      const stdioServer = server as {
        type: 'stdio'
        command: string
        args: string[]
        env?: Record<string, string>
      }

      setServerFormData({
        name: serverName,
        type: 'stdio',
        command: stdioServer.command,
        args: stdioServer.args || [],
        env: stdioServer.env,
      })
    }
  }

  const handleSaveServer = () => {
    if (!serverFormData.name.trim()) {
      toast.error('Server name is required')
      return
    }

    if (serverFormData.type === 'stdio' && !serverFormData.command?.trim()) {
      toast.error('Command is required for stdio servers')
      return
    }

    const newServerConfig: McpServerConfig =
      serverFormData.type === 'http'
        ? {
            type: 'http',
            url: serverFormData.url || '',
            headers: serverFormData.headers || {},
          }
        : {
            type: 'stdio',
            command: serverFormData.command || 'npx',
            args: (serverFormData.args || []).filter((arg) => arg.trim().length > 0),
            env: serverFormData.env || {},
          }

    const updatedServers = {
      ...baseMcpFormData,
      ...mcpUserModifications,
      [serverFormData.name]: newServerConfig,
    }

    const request: UpdateMcpSettingsRequest = {
      mcpServers: updatedServers,
    }

    updateMcpSettings.mutate(request, {
      onSuccess: () => {
        setMcpUserModifications({})
        setEditingServer(null)
        setServerFormData({
          name: '',
          type: 'stdio',
          command: 'npx',
          args: [],
          env: {},
          url: '',
          headers: {},
        })
        toast.success('MCP server saved successfully')
      },
      onError: (error) => {
        toast.error('Failed to save MCP server', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleDeleteServer = (serverName: string) => {
    const updatedServers = { ...baseMcpFormData, ...mcpUserModifications }
    delete updatedServers[serverName]

    const request: UpdateMcpSettingsRequest = {
      mcpServers: updatedServers,
    }

    updateMcpSettings.mutate(request, {
      onSuccess: () => {
        setMcpUserModifications({})
        toast.success('MCP server deleted successfully')
      },
      onError: (error) => {
        toast.error('Failed to delete MCP server', {
          description: getErrorMessage(error),
        })
      },
    })
  }

  const handleCancelEdit = () => {
    setEditingServer(null)
    setServerFormData({
      name: '',
      type: 'stdio',
      command: 'npx',
      args: [],
      env: {},
      url: '',
      headers: {},
    })
  }

  if (mcpLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (mcpError) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load MCP settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  return (
    <ConfigurationSectionWrapper>
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-medium">MCP Servers</h3>
            <p className="text-sm text-muted-foreground">
              Configure Model Context Protocol servers for Claude Code
            </p>
          </div>
          <Button onClick={handleAddServer} size="sm" variant="outline">
            <Plus className="mr-2 h-4 w-4" />
            Add Server
          </Button>
        </div>

        {/* List of configured servers */}
        <div className="space-y-2">
          {Object.keys(mcpFormData).length === 0 && !editingServer && (
            <p className="text-sm text-muted-foreground">No MCP servers configured</p>
          )}

          {Object.entries(mcpFormData).map(([serverName, config]) => {
            if (editingServer === serverName) {
              return null
            }

            const serverType = 'type' in config && config.type === 'http' ? 'HTTP' : 'stdio'

            return (
              <Card key={serverName}>
                <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                  <CardTitle className="text-sm font-medium">{serverName}</CardTitle>
                  <div className="flex gap-2">
                    <Button
                      onClick={() => handleEditServer(serverName)}
                      size="sm"
                      variant="outline"
                    >
                      Edit
                    </Button>
                    <Button
                      onClick={() => handleDeleteServer(serverName)}
                      size="sm"
                      variant="destructive"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </CardHeader>
                <CardContent>
                  <p className="text-xs text-muted-foreground">Type: {serverType}</p>
                </CardContent>
              </Card>
            )
          })}
        </div>

        {/* Server edit form */}
        {editingServer && (
          <Card>
            <CardHeader>
              <CardTitle>
                {editingServer === '__new__' ? 'Add New Server' : `Edit ${editingServer}`}
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <ConfigurationItem
                label="Server Name"
                description="Unique identifier for this MCP server"
              >
                <ConfigurationInput
                  type="text"
                  placeholder="my-server"
                  value={serverFormData.name}
                  onChange={(e) => setServerFormData((prev) => ({ ...prev, name: e.target.value }))}
                  disabled={editingServer !== '__new__'}
                />
              </ConfigurationItem>

              <ConfigurationItem label="Server Type" description="Type of MCP server connection">
                <Select
                  value={serverFormData.type}
                  onValueChange={(value: 'stdio' | 'http') =>
                    setServerFormData((prev) => ({ ...prev, type: value }))
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="stdio">stdio (Command-based)</SelectItem>
                    <SelectItem value="http">HTTP</SelectItem>
                  </SelectContent>
                </Select>
              </ConfigurationItem>

              {serverFormData.type === 'stdio' && (
                <>
                  <ConfigurationItem
                    label="Command"
                    description="Command to run the MCP server (defaults to npx)"
                  >
                    <ConfigurationInput
                      type="text"
                      placeholder="npx"
                      value={serverFormData.command || 'npx'}
                      onChange={(e) =>
                        setServerFormData((prev) => ({ ...prev, command: e.target.value }))
                      }
                    />
                  </ConfigurationItem>

                  <ConfigurationItem
                    label="Arguments"
                    description="Command arguments, e.g., package name, flags, API keys"
                  >
                    <div className="space-y-2">
                      {(serverFormData.args || []).map((arg, index) => (
                        <div key={`arg-${index}-${arg}`} className="flex gap-2">
                          <ConfigurationInput
                            type="text"
                            placeholder="@modelcontextprotocol/server-filesystem"
                            value={arg}
                            onChange={(e) => {
                              const newArgs = [...(serverFormData.args || [])]
                              newArgs[index] = e.target.value
                              setServerFormData((prev) => ({ ...prev, args: newArgs }))
                            }}
                            className="flex-1"
                          />
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            onClick={() => {
                              const newArgs = (serverFormData.args || []).filter(
                                (_, i) => i !== index,
                              )
                              setServerFormData((prev) => ({ ...prev, args: newArgs }))
                            }}
                            className="h-9 px-2"
                          >
                            <X className="h-4 w-4" />
                          </Button>
                        </div>
                      ))}
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          setServerFormData((prev) => ({
                            ...prev,
                            args: [...(prev.args || []), ''],
                          }))
                        }}
                        className="w-full"
                      >
                        <Plus className="mr-2 h-4 w-4" />
                        Add Argument
                      </Button>
                    </div>
                  </ConfigurationItem>
                </>
              )}

              {serverFormData.type === 'http' && (
                <ConfigurationItem label="URL" description="HTTP endpoint URL for the MCP server">
                  <ConfigurationInput
                    type="text"
                    placeholder="http://localhost:3000/mcp"
                    value={serverFormData.url || ''}
                    onChange={(e) =>
                      setServerFormData((prev) => ({ ...prev, url: e.target.value }))
                    }
                  />
                </ConfigurationItem>
              )}

              <div className="flex justify-end gap-2">
                <Button onClick={handleCancelEdit} variant="outline">
                  Cancel
                </Button>
                <Button onClick={handleSaveServer}>Save Server</Button>
              </div>
            </CardContent>
          </Card>
        )}
      </div>
    </ConfigurationSectionWrapper>
  )
}
