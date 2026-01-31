import type { ServerConfig, UpdateRepositoryRequest } from '@moatless/types'
import { ChevronDown, Plus, Server, Trash2 } from 'lucide-react'
import { useState } from 'react'
import {
  ConfigurationGroup,
  ConfigurationInput,
  ConfigurationItem,
  ConfigurationSwitch,
  ConfigurationTextarea,
} from '@/lib/components/configuration'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'

interface ServersGroupProps {
  formData: Partial<UpdateRepositoryRequest>
  onUpdateField: <K extends keyof UpdateRepositoryRequest>(
    field: K,
    value: UpdateRepositoryRequest[K],
  ) => void
}

/** Default file explorer server - always present, cannot be removed */
const FILE_EXPLORER_SERVER: ServerConfig = {
  name: 'files',
  label: 'File Explorer',
  port: 4000,
  builtin: true,
}

function getServers(formData: Partial<UpdateRepositoryRequest>): ServerConfig[] {
  const servers = formData.servers ?? []
  // Ensure file explorer is always present
  if (!servers.some((s) => s.name === 'files')) {
    return [FILE_EXPLORER_SERVER, ...servers]
  }
  return servers
}

function ServerCard({
  server,
  onUpdate,
  onRemove,
}: {
  server: ServerConfig
  onUpdate: (updated: ServerConfig) => void
  onRemove: () => void
}) {
  const [isOpen, setIsOpen] = useState(!server.builtin)

  return (
    <div className="rounded-lg border bg-card">
      <Collapsible open={isOpen} onOpenChange={setIsOpen}>
        <CollapsibleTrigger className="flex w-full items-center justify-between p-3 text-left hover:bg-accent/50 rounded-t-lg">
          <div className="flex items-center gap-2">
            <Server className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium text-sm">{server.label || server.name}</span>
            <span className="text-xs text-muted-foreground">Port {server.port}</span>
            {server.builtin && (
              <Badge variant="secondary" className="text-xs">
                built-in
              </Badge>
            )}
          </div>
          <div className="flex items-center gap-2">
            {!server.builtin && (
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={(e) => {
                  e.stopPropagation()
                  onRemove()
                }}
              >
                <Trash2 className="h-3.5 w-3.5 text-muted-foreground hover:text-destructive" />
              </Button>
            )}
            <ChevronDown
              className={`h-4 w-4 text-muted-foreground transition-transform ${isOpen ? 'rotate-180' : ''}`}
            />
          </div>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="space-y-4 border-t p-3">
            {server.builtin ? (
              <p className="text-sm text-muted-foreground">
                This server is built into the sandbox image and starts automatically. It cannot be
                configured or removed.
              </p>
            ) : (
              <ServerFields server={server} onUpdate={onUpdate} />
            )}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}

function ServerFields({
  server,
  onUpdate,
}: {
  server: ServerConfig
  onUpdate: (updated: ServerConfig) => void
}) {
  return (
    <>
      <ConfigurationItem label="Name" description="Unique server identifier">
        <ConfigurationInput
          value={server.name}
          onChange={(e) => onUpdate({ ...server, name: e.target.value })}
          placeholder="e.g., preview, storybook"
        />
      </ConfigurationItem>

      <ConfigurationItem label="Label" description="Human-readable name shown in the UI">
        <ConfigurationInput
          value={server.label}
          onChange={(e) => onUpdate({ ...server, label: e.target.value })}
          placeholder="e.g., Preview, Storybook"
        />
      </ConfigurationItem>

      <ConfigurationItem label="Port" description="Port the server listens on inside the sandbox">
        <ConfigurationInput
          type="number"
          min={1}
          max={65535}
          value={server.port}
          onChange={(e) => onUpdate({ ...server, port: Number(e.target.value) || 3000 })}
          placeholder="3000"
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Start Commands"
        description="Commands to start the server (one per line)"
      >
        <ConfigurationTextarea
          value={server.startCommands?.join('\n') || ''}
          onChange={(e) => {
            const commands = e.target.value.split('\n')
            onUpdate({ ...server, startCommands: commands })
          }}
          placeholder="npm install&#10;npm run dev"
          rows={3}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Startup Timeout"
        description="Maximum time to wait for server to become HTTP-responsive (seconds)"
      >
        <ConfigurationInput
          type="number"
          min={5}
          value={server.startupTimeoutSeconds ?? ''}
          onChange={(e) =>
            onUpdate({
              ...server,
              startupTimeoutSeconds: e.target.value ? Number(e.target.value) : undefined,
            })
          }
          placeholder="30"
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Auto-Reload"
        description="Server has built-in auto-reload/watch functionality"
      >
        <ConfigurationSwitch
          checked={server.autoReload ?? false}
          onCheckedChange={(checked) => onUpdate({ ...server, autoReload: checked })}
        />
      </ConfigurationItem>

      <ConfigurationItem
        label="Enable Inspector"
        description="Inject inspector script for element selection and debugging"
      >
        <ConfigurationSwitch
          checked={server.enableInspector ?? false}
          onCheckedChange={(checked) => onUpdate({ ...server, enableInspector: checked })}
        />
      </ConfigurationItem>
    </>
  )
}

function AddServerForm({
  onAdd,
  onCancel,
}: {
  onAdd: (server: ServerConfig) => void
  onCancel: () => void
}) {
  const [name, setName] = useState('')
  const [label, setLabel] = useState('')
  const [port, setPort] = useState(3000)

  const handleAdd = () => {
    if (!(name.trim() && label.trim())) {
      return
    }
    onAdd({
      name: name.trim(),
      label: label.trim(),
      port,
      enableInspector: true,
    })
  }

  return (
    <div className="rounded-lg border border-dashed p-3 space-y-3">
      <h4 className="text-sm font-medium">Add Server</h4>
      <ConfigurationItem label="Name" description="Unique identifier (e.g., preview, storybook)">
        <ConfigurationInput
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="preview"
        />
      </ConfigurationItem>
      <ConfigurationItem label="Label" description="Display name in the UI">
        <ConfigurationInput
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="Preview"
        />
      </ConfigurationItem>
      <ConfigurationItem label="Port" description="Port inside the sandbox container">
        <ConfigurationInput
          type="number"
          min={1}
          max={65535}
          value={port}
          onChange={(e) => setPort(Number(e.target.value) || 3000)}
          placeholder="3000"
        />
      </ConfigurationItem>
      <div className="flex gap-2 justify-end">
        <Button variant="outline" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" onClick={handleAdd} disabled={!(name.trim() && label.trim())}>
          Add
        </Button>
      </div>
    </div>
  )
}

export function ServersGroup({ formData, onUpdateField }: ServersGroupProps) {
  const [showAddForm, setShowAddForm] = useState(false)
  const servers = getServers(formData)

  const updateServers = (newServers: ServerConfig[]) => {
    onUpdateField('servers', newServers)
  }

  const handleUpdateServer = (index: number, updated: ServerConfig) => {
    const newServers = [...servers]
    newServers[index] = updated
    updateServers(newServers)
  }

  const handleRemoveServer = (index: number) => {
    updateServers(servers.filter((_, i) => i !== index))
  }

  const handleAddServer = (server: ServerConfig) => {
    updateServers([...servers, server])
    setShowAddForm(false)
  }

  return (
    <ConfigurationGroup title="Servers" showSeparator>
      <p className="text-sm text-muted-foreground -mt-2">
        Configure the servers that run inside the sandbox container for tasks in this repository.
      </p>

      <div className="space-y-3">
        {servers.map((server, index) => (
          <ServerCard
            key={server.name}
            server={server}
            onUpdate={(updated) => handleUpdateServer(index, updated)}
            onRemove={() => handleRemoveServer(index)}
          />
        ))}
      </div>

      {showAddForm ? (
        <AddServerForm onAdd={handleAddServer} onCancel={() => setShowAddForm(false)} />
      ) : (
        <Button variant="outline" size="sm" onClick={() => setShowAddForm(true)} className="w-full">
          <Plus className="mr-2 h-4 w-4" />
          Add Server
        </Button>
      )}
    </ConfigurationGroup>
  )
}
