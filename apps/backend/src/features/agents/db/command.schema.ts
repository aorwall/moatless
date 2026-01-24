import type { Command } from '@moatless/types'
import { boolean, index, jsonb, pgTable, text } from 'drizzle-orm/pg-core'

/**
 * Commands table schema (PostgreSQL).
 * Stores global command definitions available to all tasks.
 */
export const commands = pgTable(
  'commands',
  {
    name: text('name').primaryKey(),
    description: text('description').notNull(),
    prompt: text('prompt').notNull(),
    conditions: jsonb('conditions').$type<Command['conditions']>(),
    icon: text('icon'),
    variant: text('variant'),
    active: boolean('active'),
    willNotResume: boolean('will_not_resume'),
    agentMode: text('agent_mode'),
  },
  (table) => ({
    activeIdx: index('commands_active_idx').on(table.active),
  }),
)

/**
 * TypeScript type inferred from schema.
 * Use for type-safe insert operations.
 */
export type CommandInsert = typeof commands.$inferInsert

/**
 * TypeScript type inferred from schema.
 * Use for type-safe select operations.
 */
export type CommandSelect = typeof commands.$inferSelect

/**
 * Convert Command domain model to database insert format
 */
export function commandToInsert(command: Command): CommandInsert {
  return {
    name: command.name,
    description: command.description,
    prompt: command.prompt,
    conditions: command.conditions ?? null,
    icon: command.icon ?? null,
    variant: command.variant ?? null,
    active: command.active ?? null,
    willNotResume: command.willNotResume ?? null,
    agentMode: command.agentMode ?? null,
  }
}

/**
 * Convert database row to Command domain model
 */
export function selectToCommand(row: CommandSelect): Command {
  return {
    name: row.name,
    description: row.description,
    prompt: row.prompt,
    conditions: row.conditions ?? undefined,
    icon: row.icon ?? undefined,
    variant: row.variant as Command['variant'] | undefined,
    active: row.active ?? undefined,
    willNotResume: row.willNotResume ?? undefined,
    agentMode: (row.agentMode as Command['agentMode']) ?? undefined,
  }
}
