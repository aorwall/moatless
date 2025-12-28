import { defineConfig } from 'drizzle-kit'

/**
 * Drizzle Kit configuration for database migrations.
 *
 * Supports both SQLite (default) and PostgreSQL via DATABASE_PROVIDER env var.
 *
 * Usage:
 *   # SQLite (default)
 *   bunx drizzle-kit generate
 *
 *   # PostgreSQL
 *   DATABASE_PROVIDER=postgres DATABASE_URL=postgres://... bunx drizzle-kit generate
 */

const provider = process.env['DATABASE_PROVIDER'] || 'sqlite'

// PostgreSQL configuration
const postgresConfig = defineConfig({
  dialect: 'postgresql',
  schema: './src/**/db/pg/*.schema.ts',
  out: './drizzle-postgres',
  dbCredentials: {
    url: process.env['DATABASE_URL'] || '',
  },
  verbose: true,
  strict: true,
})

// SQLite configuration (default)
const sqliteConfig = defineConfig({
  dialect: 'sqlite',
  schema: './src/**/db/*.schema.ts',
  out: './drizzle',
  dbCredentials: {
    url: process.env['DATABASE_URL'] || './data/moatless.db',
  },
  verbose: true,
  strict: true,
})

export default provider === 'postgres' ? postgresConfig : sqliteConfig
