import { defineConfig } from 'drizzle-kit'

/**
 * Drizzle Kit configuration for PostgreSQL database migrations.
 *
 * Usage:
 *   DATABASE_URL=postgres://... bunx drizzle-kit generate
 *   DATABASE_URL=postgres://... bunx drizzle-kit push
 */
export default defineConfig({
  dialect: 'postgresql',
  schema: './src/**/db/*.schema.ts',
  out: './drizzle-postgres',
  dbCredentials: {
    url: process.env['DATABASE_URL'] || '',
  },
  verbose: true,
  strict: true,
})
