import type { Command } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { getSchemasFromEnv, type sqliteSchemas } from '../../../infrastructure/database/schemas'
import {
  commandToInsert as sqliteCommandToInsert,
  selectToCommand as sqliteSelectToCommand,
} from './command.schema'
import {
  commandToInsert as pgCommandToInsert,
  selectToCommand as pgSelectToCommand,
} from './pg/command.schema'

// Get correct schema based on DATABASE_PROVIDER
// Cast to SQLite schema type for TypeScript - runtime uses correct provider
const isPostgres = process.env['DATABASE_PROVIDER'] === 'postgres'
const schemas = getSchemasFromEnv()
const commands = schemas.commands as typeof sqliteSchemas.commands
const commandToInsert = isPostgres ? pgCommandToInsert : sqliteCommandToInsert
const selectToCommand = isPostgres ? pgSelectToCommand : sqliteSelectToCommand

/**
 * Repository for command persistence using Drizzle ORM.
 * Provides type-safe database operations for command entities.
 *
 * Commands are global entities available to all tasks and are stored
 * in a dedicated table instead of JSON files.
 */
@injectable()
export class CommandRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a command to the database.
   * Creates a new command or updates an existing one (upsert).
   *
   * @param command - Command configuration to save
   */
  async save(command: Command): Promise<void> {
    const insert = commandToInsert(command)

    await this.db.drizzle
      .insert(commands)
      .values(insert)
      .onConflictDoUpdate({
        target: commands.name,
        set: {
          description: insert.description,
          prompt: insert.prompt,
          conditions: insert.conditions,
          icon: insert.icon,
          variant: insert.variant,
          active: insert.active,
          willNotResume: insert.willNotResume,
          agentMode: insert.agentMode,
        },
      })
  }

  /**
   * Get a command by name.
   * Returns null if command not found.
   *
   * @param name - Command name identifier
   * @returns Command configuration or null if not found
   */
  async get(name: string): Promise<Command | null> {
    const rows = await this.db.drizzle
      .select()
      .from(commands)
      .where(eq(commands.name, name))
      .limit(1)

    const row = rows[0]
    if (!row) {
      return null
    }

    return selectToCommand(row)
  }

  /**
   * Get all commands.
   * Returns an array of all command configurations.
   *
   * @returns Array of all commands
   */
  async getAll(): Promise<Command[]> {
    const rows = await this.db.drizzle.select().from(commands)
    return rows.map(selectToCommand)
  }

  /**
   * List all command names.
   * Returns an array of command name identifiers.
   *
   * @returns Array of command names
   */
  async list(): Promise<string[]> {
    const rows = await this.db.drizzle.select({ name: commands.name }).from(commands)
    return rows.map((row) => row.name)
  }

  /**
   * Delete a command by name (hard delete).
   * Removes the command from the database.
   *
   * @param name - Command name identifier
   */
  async delete(name: string): Promise<void> {
    await this.db.drizzle.delete(commands).where(eq(commands.name, name))
  }

  /**
   * Check if a command exists.
   * Returns true if command exists in the database.
   *
   * @param name - Command name identifier
   * @returns True if command exists
   */
  async exists(name: string): Promise<boolean> {
    const rows = await this.db.drizzle
      .select({ name: commands.name })
      .from(commands)
      .where(eq(commands.name, name))
      .limit(1)

    return rows.length > 0
  }
}
