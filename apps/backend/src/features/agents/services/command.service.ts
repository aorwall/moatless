import type { Command, CreateCommandRequest, UpdateCommandRequest } from '@moatless/types'
import { CommandNotFoundError, ValidationError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { CommandRepository } from '../db/command.repository'

/**
 * Default commands available on fresh install.
 * These commands are automatically seeded when the database is empty.
 */
const DEFAULT_COMMANDS: Command[] = [
  {
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
  {
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
  {
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
  {
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
  {
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
  {
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
]

/**
 * Service for managing global command definitions.
 *
 * Commands are stored in the database and are available to all tasks.
 * This service provides CRUD operations and handles seeding default commands.
 */
@injectable()
export class CommandService {
  private initialized = false

  constructor(@inject(CommandRepository) private repository: CommandRepository) {}

  /**
   * Initialize the command service by seeding default commands if database is empty.
   * This is called automatically on first use.
   */
  private async initialize(): Promise<void> {
    if (this.initialized) {
      return
    }

    const existingCommands = await this.repository.list()
    if (existingCommands.length === 0) {
      // Database is empty, seed with default commands
      for (const command of DEFAULT_COMMANDS) {
        await this.repository.save(command)
      }
    }

    this.initialized = true
  }

  /**
   * Create a new command.
   *
   * @param request - Command creation request
   * @returns The created command
   * @throws ValidationError if command name already exists
   */
  async createCommand(request: CreateCommandRequest): Promise<Command> {
    await this.initialize()

    // Check if a command with the same name already exists
    const exists = await this.repository.exists(request.name)
    if (exists) {
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
      willNotResume: request.willNotResume,
      agentMode: request.agentMode,
    }

    await this.repository.save(command)
    return command
  }

  /**
   * Get a command by name.
   *
   * @param commandName - Name of the command to retrieve
   * @returns Command configuration or null if not found
   */
  async getCommand(commandName: string): Promise<Command | null> {
    await this.initialize()
    return this.repository.get(commandName)
  }

  /**
   * List all commands.
   *
   * @returns Array of all command configurations
   */
  async listCommands(): Promise<Command[]> {
    await this.initialize()
    return this.repository.getAll()
  }

  /**
   * Update an existing command.
   *
   * @param commandName - Name of the command to update
   * @param updates - Partial command updates
   * @returns The updated command
   * @throws CommandNotFoundError if command doesn't exist
   * @throws ValidationError if new name already exists
   */
  async updateCommand(commandName: string, updates: UpdateCommandRequest): Promise<Command> {
    await this.initialize()

    const command = await this.repository.get(commandName)
    if (!command) {
      throw new CommandNotFoundError(commandName)
    }

    // If name is being updated, check if it's already taken
    if (updates.name && updates.name !== commandName) {
      const nameExists = await this.repository.exists(updates.name)
      if (nameExists) {
        throw new ValidationError(`Command with name '${updates.name}' already exists`, {
          name: updates.name,
        })
      }

      // Delete old command entry (since name is the primary key)
      await this.repository.delete(commandName)
    }

    const updatedCommand: Command = {
      name: updates.name ?? command.name,
      description: updates.description ?? command.description,
      prompt: updates.prompt ?? command.prompt,
      conditions: updates.conditions ?? command.conditions,
      icon: updates.icon ?? command.icon,
      variant: updates.variant ?? command.variant,
      active: updates.active ?? command.active,
      willNotResume: updates.willNotResume ?? command.willNotResume,
      agentMode: updates.agentMode ?? command.agentMode,
    }

    await this.repository.save(updatedCommand)
    return updatedCommand
  }

  /**
   * Delete a command.
   *
   * @param commandName - Name of the command to delete
   * @throws CommandNotFoundError if command doesn't exist
   */
  async deleteCommand(commandName: string): Promise<void> {
    await this.initialize()

    const exists = await this.repository.exists(commandName)
    if (!exists) {
      throw new CommandNotFoundError(commandName)
    }

    await this.repository.delete(commandName)
  }
}
