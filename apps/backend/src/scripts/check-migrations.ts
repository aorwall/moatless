#!/usr/bin/env bun
/**
 * Diagnostic script to verify database migration status.
 * Run this on your server to check if migrations have been applied correctly.
 *
 * Usage:
 *   bun run src/scripts/check-migrations.ts
 *   # Or with custom database path:
 *   DATABASE_URL=./custom/path/moatless.db bun run src/scripts/check-migrations.ts
 */

import { Database } from 'bun:sqlite'
import { existsSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

// ANSI color codes for better readability
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
}

function log(message: string, color?: keyof typeof colors): void {
  const colorCode = color ? colors[color] : ''
  // biome-ignore lint/suspicious/noConsole: This is a CLI diagnostic script that requires console output
  console.log(`${colorCode}${message}${colors.reset}`)
}

function logSection(title: string): void {
  log(`\n${'='.repeat(60)}`, 'bright')
  log(title, 'bright')
  log('='.repeat(60), 'bright')
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Diagnostic script with sequential checks is inherently complex
function checkMigrations(): void {
  try {
    logSection('Database Migration Status Check')

    // 1. Resolve paths
    const currentDir = dirname(fileURLToPath(import.meta.url))
    const backendRoot = resolve(currentDir, '../../')
    const migrationsFolder = join(backendRoot, 'drizzle')

    log(`\nBackend Root: ${backendRoot}`, 'cyan')
    log(`Migrations Folder: ${migrationsFolder}`, 'cyan')

    // 2. Check if migrations folder exists
    checkMigrationFiles(migrationsFolder)

    // 3. Check database file
    const dbPath = process.env['DATABASE_URL'] || join(backendRoot, '../data/moatless.db')
    checkDatabaseFile(dbPath)

    // 4. Open database and check migration status
    checkDatabaseStatus(dbPath)
  } catch (error) {
    log('\n✗ Error during migration check:', 'red')
    if (error instanceof Error) {
      log(`  ${error.message}`, 'red')
      if (error.stack) {
        log(`\nStack trace:\n${error.stack}`, 'red')
      }
    } else {
      log(`  ${String(error)}`, 'red')
    }
    process.exit(1)
  }
}

function checkMigrationFiles(migrationsFolder: string): void {
  logSection('Step 1: Check Migration Files')
  if (!existsSync(migrationsFolder)) {
    log(`✗ Migrations folder NOT FOUND: ${migrationsFolder}`, 'red')
    log(
      '\nThis is a critical issue. The migrations folder must be deployed with your application.',
      'red',
    )
    process.exit(1)
  }

  log(`✓ Migrations folder exists: ${migrationsFolder}`, 'green')

  // List migration files
  try {
    const files = readdirSync(migrationsFolder)
      .filter((f) => f.endsWith('.sql'))
      .sort()

    log(`\nFound ${files.length} migration files:`, 'cyan')
    for (const file of files) {
      const filePath = join(migrationsFolder, file)
      const stats = statSync(filePath)
      log(`  - ${file} (${stats.size} bytes)`, 'blue')
    }
  } catch (error) {
    log(`✗ Error reading migrations folder: ${error}`, 'red')
  }
}

function checkDatabaseFile(dbPath: string): void {
  logSection('Step 2: Check Database File')
  log(`Database Path: ${dbPath}`, 'cyan')

  if (!existsSync(dbPath)) {
    log(`✗ Database file NOT FOUND: ${dbPath}`, 'red')
    log('\nThe database will be created on first startup.', 'yellow')
    log('Run the application once to create the database and apply migrations.', 'yellow')
    process.exit(0)
  }

  log(`✓ Database file exists: ${dbPath}`, 'green')
  const dbStats = statSync(dbPath)
  log(`  Size: ${(dbStats.size / 1024).toFixed(2)} KB`, 'blue')
}

function checkDatabaseStatus(dbPath: string): void {
  logSection('Step 3: Check Applied Migrations')
  const db = new Database(dbPath, { readonly: true })

  try {
    // Check if migrations table exists
    const tables = db
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='__drizzle_migrations'",
      )
      .all()

    let appliedCount = 0

    if (tables.length === 0) {
      log('✗ Migration tracking table (__drizzle_migrations) NOT FOUND', 'red')
      log('\nThis means migrations have never been run on this database.', 'yellow')
      log('The application should create this table and run migrations on startup.', 'yellow')
    } else {
      log('✓ Migration tracking table exists', 'green')

      // Get applied migrations
      const appliedMigrations = db
        .query<{ id: number; hash: string; created_at: number }, []>(
          'SELECT id, hash, created_at FROM __drizzle_migrations ORDER BY id',
        )
        .all()

      appliedCount = appliedMigrations.length

      if (appliedMigrations.length === 0) {
        log('\n⚠ No migrations have been applied yet', 'yellow')
      } else {
        log(`\n✓ ${appliedMigrations.length} migrations have been applied:`, 'green')
        for (const migration of appliedMigrations) {
          const date = new Date(migration.created_at)
          log(
            `  ${migration.id}. Hash: ${migration.hash.substring(0, 16)}... (${date.toISOString()})`,
            'blue',
          )
        }
      }
    }

    // 5. List all tables in database
    listDatabaseTables(db)

    // 6. Summary
    showSummary(tables.length > 0, appliedCount)
  } finally {
    db.close()
  }
}

function listDatabaseTables(db: Database): void {
  logSection('Step 4: Database Tables')
  const allTables = db
    .query<{ name: string }, []>("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
    .all()

  log(`\nFound ${allTables.length} tables in database:`, 'cyan')
  for (const table of allTables) {
    log(`  - ${table.name}`, 'blue')
  }
}

function showSummary(hasTable: boolean, appliedCount: number): void {
  logSection('Summary')
  log('✓ Migration folder: EXISTS', 'green')
  log('✓ Database file: EXISTS', 'green')

  if (hasTable) {
    log('✓ Migration system: INITIALIZED', 'green')
    log(`✓ Applied migrations: ${appliedCount}`, 'green')
  } else {
    log('⚠ Migration system: NOT INITIALIZED', 'yellow')
    log('  Action required: Start the application to run migrations', 'yellow')
  }

  log('\n✓ Database setup appears healthy', 'green')
}

// Run the check
checkMigrations()
