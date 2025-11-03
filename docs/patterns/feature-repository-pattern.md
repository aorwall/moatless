# Feature Repository Pattern

Guidance for implementing feature-scoped persistence layers using Drizzle ORM with SQLite (and future PostgreSQL support). This pattern replaces JSON file-based storage with type-safe relational database persistence while maintaining clean separation of concerns through InversifyJS dependency injection.

## Overview

Use this pattern whenever a feature needs to persist structured data (users, sessions, tasks metadata, etc.) in a relational database. Features create their own repository that relies on the `Database` infrastructure interface; InversifyJS injects the concrete Drizzle implementation configured at application startup.

**Key Difference from Storage Adapter Pattern**: This pattern uses Drizzle ORM for relational database access instead of JSON file storage, providing:
- Type-safe queries with compile-time validation
- Relational data modeling with foreign keys
- Efficient querying and indexing
- Transaction support
- Migration management

## When to Use

- ✅ Feature needs structured, relational data persistence (users, sessions, configuration)
- ✅ Data benefits from querying, filtering, or joining across entities
- ✅ You want type-safe database access with auto-completion
- ✅ Data integrity matters (foreign keys, constraints, transactions)
- ❌ Data is unstructured or document-oriented → consider keeping JSON storage
- ❌ Data is temporary or cache-like → consider in-memory solutions

## Directory Layout

```
apps/backend/
  drizzle.config.ts          # Drizzle Kit configuration
  drizzle/                   # Generated migrations
    migrations/
      0000_initial_schema.sql
      meta/
  src/
    infrastructure/
      database/
        database.ts          # Database interface + DatabaseSymbol
        drizzle.ts           # Drizzle connection factory
    features/
      <feature>/
        db/
          <domain>.schema.ts  # Domain-specific schema (e.g., user.schema.ts)
        repository/
          <domain>.repository.ts  # Feature repository using Drizzle
        services/
          <domain>.service.ts     # Business logic using repository
        index.ts
```

## Implementation Steps

1. **Install dependencies** – Add `drizzle-orm`, `drizzle-kit`, and a SQLite driver (`better-sqlite3` or `@libsql/client`)
2. **Configure Drizzle Kit** – Create `drizzle.config.ts` with schema glob pattern
3. **Create database infrastructure** – Define `Database` interface and Drizzle factory
4. **Define feature schema** – Create `<domain>.schema.ts` using Drizzle's schema builders
5. **Implement repository** – Create `<domain>.repository.ts` with type-safe CRUD operations
6. **Bind in container** – Register Database and repository implementations
7. **Generate migrations** – Run `bunx drizzle-kit generate` to create migration files
8. **Apply migrations** – Run migrations on application startup

## Setup

### 1. Install Dependencies

```bash
# Core Drizzle ORM
bun add drizzle-orm

# Development tools for migrations
bun add -D drizzle-kit

# SQLite driver (choose one)
bun add better-sqlite3        # Node.js native bindings
# OR
bun add @libsql/client        # Pure TypeScript, Bun-compatible
```

### 2. Configure Drizzle Kit

Create `drizzle.config.ts` in the backend root:

```typescript
import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  dialect: 'sqlite',
  // Glob pattern finds all domain.schema.ts files
  schema: './src/features/**/db/*.schema.ts',
  out: './drizzle',
  dbCredentials: {
    url: process.env.DB_FILE_NAME || 'data/moatless.db'
  }
})
```

This configuration:
- Uses SQLite dialect
- Discovers all `*.schema.ts` files in feature `db/` folders
- Outputs migrations to `drizzle/` directory
- Defaults to storing database in `data/moatless.db`

### 3. Environment Configuration

Add to `.env` or set in environment:

```bash
# SQLite database file path (relative to project root)
DB_FILE_NAME=data/moatless.db
```

## Infrastructure Layer

### Database Interface

The infrastructure `Database` interface provides a thin abstraction over Drizzle:

```typescript
// apps/backend/src/infrastructure/database/database.ts
import type { BetterSQLite3Database } from 'drizzle-orm/better-sqlite3'
import type { ServiceIdentifier } from 'inversify'

/**
 * Database interface for relational data persistence.
 * Provides access to Drizzle ORM instance with type-safe queries.
 */
export interface Database {
  /**
   * Get the underlying Drizzle instance for type-safe queries.
   * Use this to access all Drizzle query builder methods.
   */
  readonly drizzle: BetterSQLite3Database

  /**
   * Run pending migrations.
   * Should be called during application startup.
   */
  migrate(): Promise<void>

  /**
   * Execute operations within a database transaction.
   * Automatically commits on success, rolls back on error.
   */
  transaction<T>(fn: (tx: BetterSQLite3Database) => Promise<T>): Promise<T>

  /**
   * Close the database connection.
   * Call during graceful shutdown.
   */
  close(): Promise<void>
}

/**
 * DI symbol for injecting the Database interface.
 * Use this with @inject(DatabaseSymbol) in feature repositories.
 */
export const DatabaseSymbol: ServiceIdentifier<Database> = Symbol.for('Database')
```

### Drizzle Factory Implementation

Create the Drizzle connection factory:

```typescript
// apps/backend/src/infrastructure/database/drizzle.ts
import Database from 'better-sqlite3'
import { drizzle, type BetterSQLite3Database } from 'drizzle-orm/better-sqlite3'
import { migrate } from 'drizzle-orm/better-sqlite3/migrator'
import { injectable } from 'inversify'
import path from 'node:path'
import type { Database as DatabaseInterface } from './database'
import { logger } from '../../utils/logger'

/**
 * Drizzle-based database implementation using better-sqlite3.
 * Provides type-safe SQL access with automatic migrations.
 */
@injectable()
export class DrizzleDatabase implements DatabaseInterface {
  private readonly sqlite: Database.Database
  public readonly drizzle: BetterSQLite3Database

  constructor(dbPath: string) {
    logger.info(`Initializing database: ${dbPath}`)

    // Ensure directory exists
    const dir = path.dirname(dbPath)
    require('fs').mkdirSync(dir, { recursive: true })

    // Open SQLite database
    this.sqlite = new Database(dbPath)

    // Enable WAL mode for better concurrency
    this.sqlite.pragma('journal_mode = WAL')

    // Create Drizzle instance
    this.drizzle = drizzle(this.sqlite)
  }

  async migrate(): Promise<void> {
    logger.info('Running database migrations...')
    await migrate(this.drizzle, {
      migrationsFolder: path.join(process.cwd(), 'drizzle')
    })
    logger.info('Database migrations completed')
  }

  async transaction<T>(fn: (tx: BetterSQLite3Database) => Promise<T>): Promise<T> {
    return await this.drizzle.transaction(async (tx) => {
      return await fn(tx)
    })
  }

  async close(): Promise<void> {
    this.sqlite.close()
    logger.info('Database connection closed')
  }
}
```

## Feature Implementation

### 1. Define Schema

Create domain-specific schema files in `features/<feature>/db/<domain>.schema.ts`:

```typescript
// apps/backend/src/features/auth/db/user.schema.ts
import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Users table schema.
 * Stores authenticated user information from OAuth providers.
 */
export const users = sqliteTable('users', {
  id: text('id').primaryKey(),
  login: text('login').notNull(),
  email: text('email'),
  name: text('name'),
  avatarUrl: text('avatar_url'),
  provider: text('provider').notNull(),
  createdAt: integer('created_at', { mode: 'timestamp' }).notNull(),
  lastLoginAt: integer('last_login_at', { mode: 'timestamp' }).notNull()
})

/**
 * TypeScript type inferred from schema.
 * Use for type-safe insert operations.
 */
export type UserInsert = typeof users.$inferInsert

/**
 * TypeScript type inferred from schema.
 * Use for type-safe select operations.
 */
export type UserSelect = typeof users.$inferSelect
```

**Schema Design Tips**:
- Use `text()` for string columns, with optional `{ length: n }` for validation
- Use `integer()` for numbers, `integer({ mode: 'boolean' })` for booleans
- Use `integer({ mode: 'timestamp' })` for Date objects (stores as Unix timestamp)
- Use `integer({ mode: 'timestamp_ms' })` for millisecond precision
- Add `.notNull()` for required fields
- Add `.unique()` for unique constraints
- Export inferred types for use in repositories

### 2. Create Repository

Implement the repository using injected Database:

```typescript
// apps/backend/src/features/auth/repository/user.repository.ts
import type { User } from '@moatless/types'
import { eq } from 'drizzle-orm'
import { inject, injectable } from 'inversify'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { users } from '../db/user.schema'

/**
 * Repository for user persistence using Drizzle ORM.
 * Provides type-safe CRUD operations for user data.
 */
@injectable()
export class UserRepository {
  constructor(@inject(DatabaseSymbol) private readonly db: Database) {}

  /**
   * Save a user (insert or update on conflict).
   * Uses upsert pattern to handle both new and existing users.
   */
  async saveUser(user: User): Promise<void> {
    await this.db.drizzle
      .insert(users)
      .values({
        id: user.id,
        login: user.login,
        email: user.email,
        name: user.name,
        avatarUrl: user.avatarUrl,
        provider: user.provider,
        createdAt: user.createdAt,
        lastLoginAt: user.lastLoginAt
      })
      .onConflictDoUpdate({
        target: users.id,
        set: {
          login: user.login,
          email: user.email,
          name: user.name,
          avatarUrl: user.avatarUrl,
          lastLoginAt: user.lastLoginAt
        }
      })
  }

  /**
   * Get a user by ID.
   * Returns null if user not found.
   */
  async getUser(userId: string): Promise<User | null> {
    const result = await this.db.drizzle
      .select()
      .from(users)
      .where(eq(users.id, userId))
      .limit(1)

    return result[0] ?? null
  }

  /**
   * Get a user by email.
   * Useful for email-based login flows.
   */
  async getUserByEmail(email: string): Promise<User | null> {
    const result = await this.db.drizzle
      .select()
      .from(users)
      .where(eq(users.email, email))
      .limit(1)

    return result[0] ?? null
  }

  /**
   * List all users for a given provider.
   */
  async listUsersByProvider(provider: string): Promise<User[]> {
    return await this.db.drizzle
      .select()
      .from(users)
      .where(eq(users.provider, provider))
  }

  /**
   * Delete a user by ID.
   */
  async deleteUser(userId: string): Promise<void> {
    await this.db.drizzle
      .delete(users)
      .where(eq(users.id, userId))
  }
}
```

**Repository Patterns**:
- Use `eq()`, `ne()`, `gt()`, `gte()`, `lt()`, `lte()` from `drizzle-orm` for comparisons
- Use `and()`, `or()`, `not()` for complex conditions
- Use `.limit()` and `.offset()` for pagination
- Use `.orderBy()` for sorting
- Return `null` for not-found cases (consistent with Storage pattern)

### 3. Container Binding

Register Database and repositories in the container:

```typescript
// apps/backend/src/container/container.ts
import { DrizzleDatabase } from '../infrastructure/database/drizzle.js'
import { DatabaseSymbol } from '../infrastructure/database/database'

/**
 * Initialize database and add to container
 */
export async function initializeContainer(infrastructure: {
  // ... existing infrastructure
}): Promise<Container> {
  // Create database instance
  const dbPath = process.env.DB_FILE_NAME || 'data/moatless.db'
  const database = new DrizzleDatabase(dbPath)

  // Run migrations on startup
  await database.migrate()

  appContainer = createContainer({
    ...infrastructure,
    database
  })

  await bindServices(appContainer)
  return appContainer
}

/**
 * Bind database to container
 */
function createContainer(infrastructure: {
  database: Database
  // ... other infrastructure
}): Container {
  const container = new Container()

  // Bind database instance to DatabaseSymbol
  container.bind(DatabaseSymbol).toConstantValue(infrastructure.database)

  // ... other bindings
  return container
}

/**
 * Bind repositories
 */
export async function bindServices(container: Container): Promise<void> {
  // Import repository
  const { UserRepository } = await import('../features/auth/repository/user.repository.js')

  // Bind repository as singleton
  container.bind(UserRepository).toSelf().inSingletonScope()

  // ... other bindings
}
```

### 4. Generate and Apply Migrations

```bash
# Generate migration from schema changes
bunx drizzle-kit generate

# View pending migrations
ls drizzle/migrations/

# Migrations are automatically applied on app startup via database.migrate()
# Or manually apply using:
bunx drizzle-kit migrate
```

**If drizzle-kit generate fails due to snapshot metadata conflicts**:

Create the migration manually following this pattern (see `0007_add_workspace_runtimes_table.sql` and `0008_add_tasks_table.sql` as examples):

1. Create the SQL file: `drizzle/XXXX_descriptive_name.sql`
2. Write the CREATE TABLE statements with proper SQLite syntax
3. Add indexes using the `CREATE INDEX` statements
4. Update `drizzle/meta/_journal.json` to register the new migration:
   ```json
   {
     "idx": 8,
     "version": "6",
     "when": 1730635200000,
     "tag": "0008_add_tasks_table",
     "breakpoints": true
   }
   ```
5. Test the migration SQL with: `sqlite3 /tmp/test.db < drizzle/XXXX_*.sql`
6. Commit both the SQL file and updated journal

The migration will run automatically on next app startup.

## Query Patterns

### Basic CRUD Operations

```typescript
// Insert
await db.drizzle.insert(users).values({ id: '1', login: 'alice', ... })

// Select all
const allUsers = await db.drizzle.select().from(users)

// Select with condition
const user = await db.drizzle
  .select()
  .from(users)
  .where(eq(users.id, '1'))
  .limit(1)

// Update
await db.drizzle
  .update(users)
  .set({ lastLoginAt: new Date() })
  .where(eq(users.id, '1'))

// Delete
await db.drizzle.delete(users).where(eq(users.id, '1'))
```

### Advanced Queries

```typescript
// Multiple conditions
await db.drizzle
  .select()
  .from(users)
  .where(and(
    eq(users.provider, 'github'),
    gte(users.createdAt, startDate)
  ))

// Pagination
await db.drizzle
  .select()
  .from(users)
  .limit(20)
  .offset(40)
  .orderBy(desc(users.createdAt))

// Counting
const [{ count }] = await db.drizzle
  .select({ count: sql<number>`count(*)` })
  .from(users)
```

### Transactions

```typescript
await database.transaction(async (tx) => {
  // Both operations succeed or both fail
  await tx.insert(users).values(newUser)
  await tx.insert(sessions).values(newSession)
})
```

## Testing Guidance

### Unit Tests with Mock Database

```typescript
import { Container } from 'inversify'
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { Database } from '../../../infrastructure/database/database'
import { DatabaseSymbol } from '../../../infrastructure/database/database'
import { UserRepository } from '../user.repository'

describe('UserRepository', () => {
  let container: Container
  let userRepository: UserRepository
  let mockDb: Database

  beforeEach(() => {
    container = new Container()

    // Create mock database
    mockDb = {
      drizzle: {
        insert: vi.fn().mockReturnValue({
          values: vi.fn().mockReturnValue({
            onConflictDoUpdate: vi.fn().mockResolvedValue(undefined)
          })
        }),
        select: vi.fn().mockReturnValue({
          from: vi.fn().mockReturnValue({
            where: vi.fn().mockReturnValue({
              limit: vi.fn().mockResolvedValue([mockUser])
            })
          })
        })
      },
      migrate: vi.fn(),
      transaction: vi.fn(),
      close: vi.fn()
    } as unknown as Database

    // Bind mocks
    container.bind(DatabaseSymbol).toConstantValue(mockDb)
    container.bind(UserRepository).toSelf()

    // Resolve repository
    userRepository = container.get(UserRepository)
  })

  afterEach(() => {
    container.unbindAll()
  })

  it('should save user with correct data', async () => {
    const user = {
      id: '1',
      login: 'alice',
      email: 'alice@example.com',
      // ... other fields
    }

    await userRepository.saveUser(user)

    expect(mockDb.drizzle.insert).toHaveBeenCalled()
  })
})
```

### Integration Tests with In-Memory SQLite

```typescript
import Database from 'better-sqlite3'
import { drizzle } from 'drizzle-orm/better-sqlite3'
import { migrate } from 'drizzle-orm/better-sqlite3/migrator'
import { describe, it, expect, beforeEach } from 'vitest'
import { DrizzleDatabase } from '../../../infrastructure/database/drizzle'
import { UserRepository } from '../user.repository'

describe('UserRepository Integration', () => {
  let database: DrizzleDatabase
  let userRepository: UserRepository

  beforeEach(async () => {
    // Create in-memory database
    database = new DrizzleDatabase(':memory:')
    await database.migrate()

    userRepository = new UserRepository(database)
  })

  it('should save and retrieve user', async () => {
    const user = {
      id: '1',
      login: 'alice',
      email: 'alice@example.com',
      name: 'Alice',
      avatarUrl: null,
      provider: 'github',
      createdAt: new Date(),
      lastLoginAt: new Date()
    }

    await userRepository.saveUser(user)
    const retrieved = await userRepository.getUser('1')

    expect(retrieved).toEqual(user)
  })
})
```

## Migration from Storage to Repository

When migrating an existing feature from JSON file storage to Drizzle:

### 1. Create Schema and Repository

Follow the implementation steps above to create:
- `features/<feature>/db/<domain>.schema.ts`
- `features/<feature>/repository/<domain>.repository.ts`

### 2. Run Both Systems in Parallel (Gradual Migration)

During transition, keep both storage and repository implementations:

```typescript
// Service can use both during migration
@injectable()
export class UserService {
  constructor(
    @inject(UserStorage) private readonly storage: UserStorage,  // Legacy
    @inject(UserRepository) private readonly repository: UserRepository  // New
  ) {}

  async getUser(id: string): Promise<User | null> {
    // Try repository first, fall back to storage
    const user = await this.repository.getUser(id)
    if (user) return user

    return await this.storage.getUser(id)
  }

  async saveUser(user: User): Promise<void> {
    // Write to both during migration
    await Promise.all([
      this.storage.saveUser(user),
      this.repository.saveUser(user)
    ])
  }
}
```

### 3. Data Migration Script (Optional)

Create a one-time migration script to move data:

```typescript
// scripts/migrate-users-to-db.ts
import { appContainer } from '../src/container/container'
import { UserStorage } from '../src/features/auth/storage/user.storage'
import { UserRepository } from '../src/features/auth/repository/user.repository'

async function migrateUsers() {
  const storage = appContainer.get(UserStorage)
  const repository = appContainer.get(UserRepository)

  // List all users from JSON storage
  const userIds = await storage.list('users/')

  for (const userId of userIds) {
    const user = await storage.getUser(userId)
    if (user) {
      await repository.saveUser(user)
      console.log(`Migrated user ${userId}`)
    }
  }
}

migrateUsers().catch(console.error)
```

### 4. Remove Storage Implementation

Once migration is complete:
1. Update services to only use repository
2. Remove storage injections
3. Delete or deprecate storage files
4. Remove JSON data files

## PostgreSQL Support (Future)

To switch to PostgreSQL in the future:

### 1. Update Dependencies

```bash
bun remove better-sqlite3
bun add pg
```

### 2. Update drizzle.config.ts

```typescript
export default defineConfig({
  dialect: 'postgresql',  // Changed from 'sqlite'
  schema: './src/features/**/db/*.schema.ts',
  out: './drizzle',
  dbCredentials: {
    url: process.env.DATABASE_URL  // PostgreSQL connection string
  }
})
```

### 3. Update Schema Files

```typescript
// Change imports from sqlite-core to pg-core
import { pgTable, text, timestamp, varchar } from 'drizzle-orm/pg-core'

export const users = pgTable('users', {
  id: text('id').primaryKey(),
  login: varchar('login', { length: 256 }).notNull(),
  email: varchar('email', { length: 256 }),
  name: varchar('name', { length: 256 }),
  avatarUrl: text('avatar_url'),
  provider: varchar('provider', { length: 50 }).notNull(),
  createdAt: timestamp('created_at').notNull().defaultNow(),
  lastLoginAt: timestamp('last_login_at').notNull()
})
```

### 4. Update Drizzle Factory

```typescript
import { drizzle } from 'drizzle-orm/node-postgres'
import { Pool } from 'pg'

export class DrizzleDatabase implements DatabaseInterface {
  private readonly pool: Pool
  public readonly drizzle: NodePgDatabase

  constructor(connectionString: string) {
    this.pool = new Pool({ connectionString })
    this.drizzle = drizzle(this.pool)
  }
}
```

## Related Docs

- `docs/patterns/feature-storage-adapters.md` – JSON file-based storage pattern (legacy)
- `docs/patterns/dependency-injection.md` – InversifyJS usage patterns
- `docs/patterns/backend-feature-architecture.md` – Overall feature layout
- [Drizzle ORM Documentation](https://orm.drizzle.team) – Official Drizzle docs
- [Drizzle Kit Documentation](https://orm.drizzle.team/kit-docs/overview) – Migration tools
