import { CommandNotFoundError, ValidationError } from '@moatless/types'
import type {
  Command,
  CommandsFile,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@moatless/types/types/command'
import type { StorageBackend } from './storage.js'

const COMMANDS_FILE = 'commands'

const DEFAULT_COMMANDS: CommandsFile = {
  commands: {
    commit: {
      name: 'commit',
      description: 'Commit the changes to the repository',
      prompt: 'commit the relevant changes',
      conditions: {
        requiresUncommittedChanges: true,
        requiresTaskNotRunning: true,
      },
      icon: 'GitCommit',
      variant: 'outline',
    },
    continue: {
      name: 'continue',
      description: 'Continue after an error',
      prompt: 'continue with the task',
      conditions: {
        requiresLastIterationError: true,
        requiresTaskNotRunning: true,
      },
      icon: 'Play',
      variant: 'default',
    },
    'create-pr': {
      name: 'create-pr',
      description: 'Create a pull request',
      prompt: 'create a pull request with the changes',
      conditions: {
        requiresAheadOfMain: true,
        requiresTaskNotRunning: true,
        allowedGitSyncStatuses: ['ahead'],
      },
      icon: 'GitPullRequest',
      variant: 'outline',
    },
    'merge-to-main': {
      name: 'merge-to-main',
      description: 'Merge changes to main branch',
      prompt: 'merge the changes to the main branch',
      conditions: {
        requiresCommittedChanges: true,
        requiresTaskNotRunning: true,
        allowedGitSyncStatuses: ['ahead'],
      },
      icon: 'GitMerge',
      variant: 'outline',
    },
    'sync-with-main': {
      name: 'sync-with-main',
      description: 'Sync branch with main',
      prompt: 'sync the branch with the latest changes from main',
      conditions: {
        requiresBehindMain: true,
        requiresTaskNotRunning: true,
        allowedGitSyncStatuses: ['behind', 'diverged'],
      },
      icon: 'GitBranch',
      variant: 'secondary',
    },
    compact: {
      name: 'compact',
      description: 'Summarize the conversation and compact the context',
      prompt:
        "Please provide a comprehensive summary of our conversation so far, including:\n\n1. The main task or problem we're working on\n2. Key decisions and approaches taken\n3. Important code changes made\n4. Current status and any remaining work\n5. Any important context that should be preserved\n\nThis summary will be used as context for future interactions, so please be thorough and include all relevant details.",
      conditions: {
        requiresTaskNotRunning: true,
      },
      icon: 'FileText',
      variant: 'secondary',
      willNotResume: true,
    },
  },
}

export class CommandService {
  constructor(private storage: StorageBackend) {}

  private async loadCommands(): Promise<CommandsFile> {
    try {
      const data = await this.storage.get<CommandsFile>(COMMANDS_FILE)
      return data || DEFAULT_COMMANDS
    } catch {
      // File doesn't exist, create it with default commands
      await this.storage.save(COMMANDS_FILE, DEFAULT_COMMANDS)
      return DEFAULT_COMMANDS
    }
  }

  private async saveCommands(commands: CommandsFile): Promise<void> {
    await this.storage.save(COMMANDS_FILE, commands)
  }

  async createCommand(request: CreateCommandRequest): Promise<Command> {
    const commandsFile = await this.loadCommands()

    // Check if a command with the same name already exists
    if (commandsFile.commands[request.name]) {
      throw new ValidationError(`Command with name '${request.name}' already exists`, {
        name: request.name,
      })
    }

    const command: Command = {
      name: request.name,
      description: request.description,
      prompt: request.prompt,
      conditions: request.conditions,
      icon: request.icon,
      variant: request.variant,
      active: request.active,
    }

    commandsFile.commands[request.name] = command
    await this.saveCommands(commandsFile)
    return command
  }

  async getCommand(commandName: string): Promise<Command | null> {
    const commandsFile = await this.loadCommands()
    return commandsFile.commands[commandName] || null
  }

  async listCommands(): Promise<Command[]> {
    const commandsFile = await this.loadCommands()
    return Object.values(commandsFile.commands)
  }

  async updateCommand(commandName: string, updates: UpdateCommandRequest): Promise<Command> {
    const commandsFile = await this.loadCommands()
    const command = commandsFile.commands[commandName]

    if (!command) {
      throw new CommandNotFoundError(commandName)
    }

    // If name is being updated, check if it's already taken
    if (updates.name && updates.name !== commandName) {
      if (commandsFile.commands[updates.name]) {
        throw new ValidationError(`Command with name '${updates.name}' already exists`, {
          name: updates.name,
        })
      }
      // Remove old command entry
      delete commandsFile.commands[commandName]
    }

    const updatedCommand: Command = {
      ...command,
      ...updates,
    }

    // Save with the new name (or same name if not updated)
    const newName = updates.name || commandName
    commandsFile.commands[newName] = updatedCommand
    await this.saveCommands(commandsFile)
    return updatedCommand
  }

  async deleteCommand(commandName: string): Promise<void> {
    const commandsFile = await this.loadCommands()

    if (!commandsFile.commands[commandName]) {
      throw new CommandNotFoundError(commandName)
    }

    delete commandsFile.commands[commandName]
    await this.saveCommands(commandsFile)
  }
}

export function createCommandService(storage: StorageBackend): CommandService {
  return new CommandService(storage)
}
