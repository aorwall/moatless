import type { Command } from '@moatless/shared/types/command'
import { Link } from '@tanstack/react-router'
import { Plus, Trash2 } from 'lucide-react'
import { useDeleteCommand } from '@/features/commands/hooks/use-commands'
import { Button } from '@/lib/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

interface CommandListProps {
  commands?: Array<Command>
  isLoading: boolean
}

export function CommandList({ commands, isLoading }: CommandListProps) {
  const deleteCommand = useDeleteCommand()

  const handleDelete = (commandName: string, event: React.MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()
    if (confirm(`Are you sure you want to delete the command "${commandName}"?`)) {
      deleteCommand.mutate(commandName)
    }
  }

  if (isLoading) {
    return <div className="text-center py-8 text-muted-foreground">Loading commands...</div>
  }

  if (!commands || commands.length === 0) {
    return (
      <div className="text-center py-8 space-y-4">
        <div className="text-muted-foreground">
          <p>No commands configured yet.</p>
          <p className="text-sm mt-2">Create your first command to get started.</p>
        </div>
        <Button asChild>
          <Link to="/commands/new">
            <Plus className="h-4 w-4 mr-2" />
            New Command
          </Link>
        </Button>
      </div>
    )
  }

  return (
    <TooltipProvider>
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-2xl font-bold">Commands</h2>
            <p className="text-muted-foreground">Manage commands for the code agent</p>
          </div>
          <Button asChild>
            <Link to="/commands/new">
              <Plus className="h-4 w-4 mr-2" />
              New Command
            </Link>
          </Button>
        </div>
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[25%]">Name</TableHead>
                <TableHead className="w-[35%]">Description</TableHead>
                <TableHead className="w-[30%]">Prompt</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {commands.map((command) => (
                <TableRow key={command.name} className="cursor-pointer">
                  <TableCell className="font-medium">
                    <Link
                      to="/commands/$commandName"
                      params={{ commandName: command.name }}
                      className="block hover:text-primary"
                    >
                      {command.name}
                    </Link>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="truncate block max-w-xs">{command.description}</span>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p className="text-xs max-w-sm">{command.description}</p>
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="truncate block max-w-xs font-mono text-sm">
                          {command.prompt}
                        </span>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p className="text-xs font-mono max-w-sm">{command.prompt}</p>
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-2">
                      <Button variant="ghost" size="sm" asChild>
                        <Link to="/commands/$commandName" params={{ commandName: command.name }}>
                          Edit
                        </Link>
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={(e) => handleDelete(command.name, e)}
                        disabled={deleteCommand.isPending}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </div>
    </TooltipProvider>
  )
}
