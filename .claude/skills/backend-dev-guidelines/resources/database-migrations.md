# Database Migrations

## Overview

This project uses Drizzle ORM for database schema management and migrations. Migrations are SQL files that describe incremental changes to the database schema.

**Key locations:**
- Migration files: `apps/backend/drizzle/*.sql`
- Migration metadata: `apps/backend/drizzle/meta/_journal.json`
- Migration runner: `apps/backend/src/infrastructure/database/drizzle.ts`
- Schema definitions: `apps/backend/src/features/*/db/*.schema.ts`

## Creating Migrations

**Always use `drizzle-kit generate`** to create migrations. Never hand-write migration files.

```bash
# Generate migration from schema changes
cd apps/backend
bunx drizzle-kit generate
```

This ensures:
- Correct SQL syntax
- Proper statement breakpoints
- Metadata updates in `_journal.json`
- Snapshot generation

## Migration File Format

Drizzle requires the `--> statement-breakpoint` marker between SQL statements.

### ✅ Correct Format

```sql
-- Add createdBy columns for user-scoped access control
ALTER TABLE `repositories` ADD COLUMN `created_by` TEXT;--> statement-breakpoint
ALTER TABLE `tasks` ADD COLUMN `created_by` TEXT;--> statement-breakpoint
CREATE INDEX `repositories_created_by_idx` ON `repositories`(`created_by`);--> statement-breakpoint
CREATE INDEX `tasks_created_by_idx` ON `tasks`(`created_by`);
```

### ❌ Incorrect Format (Will Fail Silently)

```sql
-- Missing statement-breakpoint markers
ALTER TABLE repositories ADD COLUMN created_by TEXT;

ALTER TABLE tasks ADD COLUMN created_by TEXT;

CREATE INDEX repositories_created_by_idx ON repositories(created_by);
CREATE INDEX tasks_created_by_idx ON tasks(created_by);
```

**What happens without statement-breakpoint:**
- Only the FIRST SQL statement executes
- Subsequent statements are silently skipped
- Migration is marked as "applied" even though it's incomplete
- No error is thrown

## Critical Requirements

### 1. Statement Breakpoints Are Mandatory

Drizzle uses `--> statement-breakpoint` to parse multiple SQL statements in a migration file. Without this marker, only the first statement executes.

**Real-world example from migration 0000:**

```sql
CREATE TABLE `tasks` (
  `id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
  `task_id` text NOT NULL,
  `status` text NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `tasks_task_id_idx` ON `tasks` (`task_id`);--> statement-breakpoint
CREATE INDEX `tasks_status_idx` ON `tasks` (`status`);--> statement-breakpoint
```

### 2. Always Await migrate()

The migration runner MUST await the `migrate()` call to ensure migrations complete before the application starts.

**Correct implementation:**

```typescript
// apps/backend/src/infrastructure/database/drizzle.ts
async migrate(): Promise<void> {
  // ... path resolution ...

  // Run migrations - MUST await!
  await migrate(this.drizzle, { migrationsFolder })

  // Check final status
  const finalMigrations = this.sqlite
    .query('SELECT id FROM __drizzle_migrations ORDER BY id')
    .all()
  this.logger.info({ totalApplied: finalMigrations.length }, '✓ Database migrations completed')
}
```

**What happens without await:**
- Race condition: migration status checked before migrations finish
- Application starts with incomplete schema
- Queries fail with "no such column" errors
- Misleading success logs

### 3. Build Process Must Copy Migration Files

Production deployments run from `dist/` folder. Migration files must be copied during build.

**package.json:**

```json
{
  "scripts": {
    "build": "tsc -p tsconfig.build.json && bun run copy:assets",
    "copy:assets": "mkdir -p dist/drizzle && cp -r drizzle/* dist/drizzle/"
  }
}
```

**Migration runner path resolution:**

```typescript
// Tries multiple paths to support both dev and production
const possiblePaths = [
  resolve(currentDir, '../drizzle'),      // Production: dist/drizzle
  resolve(currentDir, '../../../drizzle'), // Development: apps/backend/drizzle
]
```

**Why this matters:**
- Development runs from source: `bun src/index.ts` (uses fallback path)
- Production runs from build: `bun dist/index.js` (needs dist/drizzle/)
- Without copying, production migrations fail silently

## Testing Migrations

### Fresh Database Test

Always test migrations on a fresh database before deploying:

```typescript
// apps/backend/src/infrastructure/database/__tests__/migrations.integration.test.ts
import { describe, test, expect, beforeAll } from 'vitest'
import { Database } from 'bun:sqlite'
import { DrizzleDatabase } from '../drizzle'
import { tasks } from '../../features/tasks/db/task.schema'

describe('Database Migrations', () => {
  test('all migrations apply to fresh database', async () => {
    const dbPath = ':memory:'
    const db = new DrizzleDatabase(dbPath)

    // Run migrations on empty database
    await db.migrate()

    // Verify migration count
    const migrations = db.drizzle
      .prepare('SELECT COUNT(*) as count FROM __drizzle_migrations')
      .get() as { count: number }

    expect(migrations.count).toBe(12) // Update when adding migrations
  })

  test('schema matches expected structure', async () => {
    const dbPath = ':memory:'
    const db = new DrizzleDatabase(dbPath)
    await db.migrate()

    // Verify critical columns exist
    const taskSchema = db.drizzle
      .prepare('PRAGMA table_info(tasks)')
      .all() as Array<{ name: string }>

    const columnNames = taskSchema.map(col => col.name)
    expect(columnNames).toContain('task_id')
    expect(columnNames).toContain('status')
    expect(columnNames).toContain('created_by') // Added in migration 0009
  })
})
```

### Production Build Test

Test the built application before deploying:

```bash
# Build and test production bundle
bun run build

# Verify drizzle folder was copied
ls -la apps/backend/dist/drizzle/

# Run production build with test database
DATA_DIR=/tmp/test-migration NODE_ENV=production bun apps/backend/dist/index.js

# Verify schema
sqlite3 /tmp/test-migration/moatless.db "PRAGMA table_info(tasks);"
```

## Anti-Patterns & Common Pitfalls

### ❌ Hand-Writing Migrations

```sql
-- DON'T: Hand-written migration (missing statement-breakpoint)
ALTER TABLE tasks ADD COLUMN created_by TEXT;
CREATE INDEX tasks_created_by_idx ON tasks(created_by);
```

**Problem:**
- Easy to forget statement-breakpoint markers
- No snapshot generated
- Manual SQL prone to errors

**Solution:**
```bash
# DO: Always use drizzle-kit generate
bunx drizzle-kit generate
```

### ❌ Missing Statement-Breakpoint

```sql
-- DON'T: Multiple statements without breakpoint
ALTER TABLE repositories ADD COLUMN created_by TEXT;
ALTER TABLE tasks ADD COLUMN created_by TEXT;
```

**Problem:**
- Only first ALTER TABLE executes
- Second statement silently skipped
- Migration marked as applied despite being incomplete

**Solution:**
```sql
-- DO: Use statement-breakpoint between statements
ALTER TABLE `repositories` ADD COLUMN `created_by` TEXT;--> statement-breakpoint
ALTER TABLE `tasks` ADD COLUMN `created_by` TEXT;--> statement-breakpoint
```

### ❌ Not Awaiting migrate()

```typescript
// DON'T: Missing await
async migrate(): Promise<void> {
  migrate(this.drizzle, { migrationsFolder }) // Race condition!
  const count = this.sqlite.query('SELECT COUNT(*) FROM __drizzle_migrations').get()
}
```

**Problem:**
- Migration count checked before migrations finish
- Application starts with incomplete schema
- "no such column" errors at runtime

**Solution:**
```typescript
// DO: Always await
async migrate(): Promise<void> {
  await migrate(this.drizzle, { migrationsFolder })
  const count = this.sqlite.query('SELECT COUNT(*) FROM __drizzle_migrations').get()
}
```

### ❌ Testing Only Incremental Migrations

```bash
# DON'T: Only test with existing database
bun run dev  # Uses existing ~/.moatless/moatless.db
```

**Problem:**
- Your dev database migrated incrementally (0000→0001→0002...)
- Production may apply all at once (0000→0012)
- Fresh database bugs not caught until production

**Solution:**
```bash
# DO: Test on fresh database
rm -f /tmp/test.db
DATA_DIR=/tmp bun run dev  # Forces fresh migration
```

### ❌ Not Testing Production Build

```bash
# DON'T: Only test development mode
bun run dev  # Uses source files
```

**Problem:**
- Dev uses source `drizzle/` folder (fallback path works)
- Production uses `dist/drizzle/` folder
- Build issues not caught until production deployment

**Solution:**
```bash
# DO: Test production build locally
bun run build
NODE_ENV=production bun dist/index.js
```

## Production Deployment Checklist

Before deploying to production:

- [ ] Run `bun run build` successfully
- [ ] Verify `dist/drizzle/` folder exists with all migration files
- [ ] Verify migration count: `ls dist/drizzle/*.sql | wc -l`
- [ ] Test production build locally:
  ```bash
  DATA_DIR=/tmp/prod-test NODE_ENV=production bun dist/index.js
  ```
- [ ] Verify fresh database migration works:
  ```bash
  sqlite3 /tmp/prod-test/moatless.db "SELECT COUNT(*) FROM __drizzle_migrations;"
  ```
- [ ] Check for "no such column" errors in startup logs
- [ ] Verify expected columns exist:
  ```bash
  sqlite3 /tmp/prod-test/moatless.db "PRAGMA table_info(tasks);"
  ```

## Troubleshooting

### Migration says "applied" but column missing

**Symptoms:**
- Log shows "12 migrations completed"
- Runtime error: "no such column: tasks.created_by"
- Query shows 12 rows in `__drizzle_migrations`

**Cause:**
- Missing `--> statement-breakpoint` in migration file
- Only first statement executed, rest silently skipped

**Fix:**
1. Add statement-breakpoint markers to migration file
2. Rebuild: `bun run build`
3. Test on fresh database
4. Manually add column to existing databases:
   ```sql
   ALTER TABLE tasks ADD COLUMN created_by TEXT;
   CREATE INDEX tasks_created_by_idx ON tasks(created_by);
   ```

### Migrations not found in production

**Symptoms:**
- Error: "Migration folder not found"
- Paths checked: `dist/drizzle`, `apps/backend/drizzle`

**Cause:**
- Migration files not copied during build

**Fix:**
1. Update `package.json` copy-assets script:
   ```json
   "copy:assets": "mkdir -p dist/drizzle && cp -r drizzle/* dist/drizzle/"
   ```
2. Rebuild: `bun run build`
3. Verify: `ls dist/drizzle/`

### Race condition: migrations incomplete at startup

**Symptoms:**
- Intermittent "no such table" or "no such column" errors
- Works sometimes, fails other times

**Cause:**
- `migrate()` not awaited in migration runner

**Fix:**
```typescript
// Add await
await migrate(this.drizzle, { migrationsFolder })
```

## Best Practices

1. **Always use drizzle-kit generate** - Never hand-write migrations
2. **Test on fresh database** - Don't rely only on incremental migrations
3. **Test production build** - Verify migrations work from dist/
4. **Verify schema** - Check actual database columns after migration
5. **Use descriptive names** - Migration names should explain what changed
6. **Keep migrations atomic** - Each migration should be a logical unit
7. **Never edit applied migrations** - Create new migration to fix issues
8. **Document breaking changes** - Add comments explaining schema changes

## References

- [Drizzle ORM Migrations](https://orm.drizzle.team/docs/migrations)
- [drizzle-kit CLI](https://orm.drizzle.team/kit-docs/overview)
- Migration runner: `apps/backend/src/infrastructure/database/drizzle.ts`
- Example migration: `apps/backend/drizzle/0000_lively_vertigo.sql`
