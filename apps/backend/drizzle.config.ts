import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  // SQLite dialect
  dialect: 'sqlite',

  // Database connection
  dbCredentials: {
    url: process.env['DATABASE_URL'] || './data/moatless.db',
  },

  // Schema files - glob pattern to match all domain-named schemas in feature directories
  schema: './src/**/db/*.schema.ts',

  // Output directory for migrations
  out: './drizzle',

  // Verbose logging for debugging
  verbose: true,

  // Strict mode for type safety
  strict: true,
})
