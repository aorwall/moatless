#!/usr/bin/env bun
// biome-ignore-all lint/suspicious/noConsole: CLI script uses console for output
/**
 * CLI script to promote a user to admin role.
 *
 * Usage:
 *   bun run apps/backend/src/scripts/promote-admin.ts <userId>
 *   bun run apps/backend/src/scripts/promote-admin.ts --list
 *
 * Examples:
 *   # Promote a specific user
 *   bun run apps/backend/src/scripts/promote-admin.ts github|12345
 *
 *   # List all users
 *   bun run apps/backend/src/scripts/promote-admin.ts --list
 */

import { join } from 'node:path'

async function main() {
  const args = process.argv.slice(2)

  if (args.length === 0 || args[0] === '--help' || args[0] === '-h') {
    console.log(`
Usage: bun run apps/backend/src/scripts/promote-admin.ts <userId>
       bun run apps/backend/src/scripts/promote-admin.ts --list

Options:
  --list    List all users with their roles
  --help    Show this help message

Examples:
  bun run apps/backend/src/scripts/promote-admin.ts github|12345
  bun run apps/backend/src/scripts/promote-admin.ts --list
`)
    process.exit(0)
  }

  // Initialize database
  const { createDatabase, getDatabaseConfig } = await import(
    '../infrastructure/database/factory.js'
  )
  const { getDataDir } = await import('../utils/data-dir.js')

  const dbConfig = getDatabaseConfig()
  if (dbConfig.provider === 'sqlite' && !dbConfig.sqlitePath) {
    const dataDir = getDataDir()
    dbConfig.sqlitePath = join(dataDir, 'moatless.db')
  }

  const database = createDatabase(dbConfig)
  await database.migrate()

  // Import UserRepository dependencies
  const { UserRepository } = await import('../features/auth/db/user.repository.js')

  // Create repository instance manually (no DI container)
  const userRepo = new UserRepository(database)

  // args[0] is guaranteed to exist since we check args.length above
  const command = args[0] as string

  if (command === '--list') {
    const users = await userRepo.getAllUsers()
    if (users.length === 0) {
      console.log('No users found.')
      process.exit(0)
    }

    console.log('\nUsers:')
    console.log('─'.repeat(80))
    for (const user of users) {
      const roleTag = user.role === 'admin' ? '[ADMIN]' : '[user]'
      console.log(`${roleTag} ${user.id} - ${user.login} (${user.email || 'no email'})`)
    }
    console.log('─'.repeat(80))
    console.log(`Total: ${users.length} users`)
    process.exit(0)
  }

  const userId = command

  // Get user
  const user = await userRepo.getUser(userId)
  if (!user) {
    console.error(`Error: User not found: ${userId}`)
    console.log('\nUse --list to see all users.')
    process.exit(1)
  }

  if (user.role === 'admin') {
    console.log(`User ${user.login} (${userId}) is already an admin.`)
    process.exit(0)
  }

  // Promote to admin
  const success = await userRepo.updateRole(userId, 'admin')
  if (success) {
    console.log(`✓ User ${user.login} (${userId}) has been promoted to admin.`)
  } else {
    console.error('Error: Failed to update user role.')
    process.exit(1)
  }
}

main().catch((error) => {
  console.error('Error:', error)
  process.exit(1)
})
