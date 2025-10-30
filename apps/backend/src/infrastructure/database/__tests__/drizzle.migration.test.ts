import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { logger } from '../../../utils/logger'
import { createDrizzleDatabase } from '../drizzle'

async function applyLegacyMigrations(dbPath: string): Promise<void> {
  const database = createDrizzleDatabase(dbPath, logger)
  const sqlite = (database as unknown as { sqlite: { exec: (sql: string) => void } }).sqlite

  const migrationFiles = [
    '0000_black_penance.sql',
    '0001_uneven_inertia.sql',
    '0002_unknown_phil_sheldon.sql',
    '0003_cultured_the_fallen.sql',
  ]

  for (const file of migrationFiles) {
    const sql = await fs.readFile(new URL(`../../../drizzle/${file}`, import.meta.url), 'utf8')
    sqlite.exec(sql)
  }

  sqlite.exec(
    `INSERT INTO task_messages (
        task_id,
        uuid,
        message_type,
        user_type,
        timestamp,
        message,
        created_at
      ) VALUES (
        'task-legacy',
        'legacy-user-1',
        'user',
        'user',
        '2024-01-01T00:00:00.000Z',
        '{"role":"user","content":"Legacy user message"}',
        strftime('%s','now')
      ), (
        'task-legacy',
        'legacy-assistant-1',
        'assistant',
        'assistant',
        '2024-01-01T00:00:01.000Z',
        '{"type":"message","role":"assistant","content":[{"type":"text","text":"Legacy assistant"}],"usage":{"input_tokens":10,"output_tokens":20}}',
        strftime('%s','now')
      );`,
  )

  await database.close()
}

describe('DrizzleDatabase legacy schema normalization', () => {
  let tempDir: string
  let dbPath: string

  beforeEach(async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'drizzle-legacy-'))
    dbPath = path.join(tempDir, 'messages.db')
    await applyLegacyMigrations(dbPath)
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  it('upgrades task_messages table that lacks extracted message columns', async () => {
    const database = createDrizzleDatabase(dbPath, logger)

    await database.migrate()
    await database.close()

    const verificationDb = createDrizzleDatabase(dbPath, logger)
    const sqlite = (
      verificationDb as unknown as {
        sqlite: { prepare: (sql: string) => { all: () => unknown[]; get: () => unknown } }
      }
    ).sqlite

    const columns = sqlite.prepare("PRAGMA table_info('task_messages')").all() as { name: string }[]
    expect(columns.map((col) => col.name)).toContain('message_role')
    expect(columns.map((col) => col.name)).toContain('message_content')

    const row = sqlite
      .prepare(
        `SELECT message_role as role, message_content as content FROM task_messages WHERE uuid = 'legacy-user-1'`,
      )
      .get() as { role: string; content: string }

    expect(row.role).toBe('user')
    expect(JSON.parse(row.content)).toBe('Legacy user message')

    const assistantRow = sqlite
      .prepare(
        `SELECT message_role as role, message_content as content, assistant_usage_input_tokens as inputTokens FROM task_messages WHERE uuid = 'legacy-assistant-1'`,
      )
      .get() as { role: string; content: string; inputTokens: number }

    expect(assistantRow.role).toBe('assistant')
    expect(JSON.parse(assistantRow.content)).toEqual([{ type: 'text', text: 'Legacy assistant' }])
    expect(assistantRow.inputTokens).toBe(10)

    await verificationDb.close()
  })
})
