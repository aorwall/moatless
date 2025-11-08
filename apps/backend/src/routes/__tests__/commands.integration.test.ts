import type { Command } from '@moatless/types'
import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest'
import { createApp } from '../../app'
import { appContainer } from '../../container/container'
import { CommandRepository } from '../../features/agents/db/command.repository'

describe('Commands Routes Integration', () => {
  let app: Awaited<ReturnType<typeof createApp>>['app']
  let repository: CommandRepository

  beforeAll(async () => {
    const result = await createApp()
    app = result.app
    repository = appContainer.get(CommandRepository)
  })

  beforeEach(async () => {
    // Clear only non-default commands before each test
    const allCommands = await repository.getAll()
    const defaultCommandNames = [
      'commit',
      'continue',
      'create-pr',
      'merge-to-main',
      'sync-with-main',
      'compact',
    ]
    for (const command of allCommands) {
      if (!defaultCommandNames.includes(command.name)) {
        await repository.delete(command.name)
      }
    }
  })

  afterAll(async () => {
    // Cleanup - remove test commands
    const allCommands = await repository.getAll()
    for (const command of allCommands) {
      await repository.delete(command.name)
    }
  })

  describe('Default Commands Seeding', () => {
    it('should seed default commands on first list request', async () => {
      const response = await app.request('/api/v1/commands', {
        method: 'GET',
      })

      expect(response.status).toBe(200)
      const body = (await response.json()) as { commands: Command[] }

      // Should have default commands
      expect(body.commands.length).toBeGreaterThan(0)
      const commandNames = body.commands.map((c) => c.name)
      expect(commandNames).toContain('commit')
      expect(commandNames).toContain('continue')
      expect(commandNames).toContain('create-pr')
      expect(commandNames).toContain('merge-to-main')
      expect(commandNames).toContain('sync-with-main')
      expect(commandNames).toContain('compact')
    })
  })

  describe('POST /api/v1/commands', () => {
    it('should create a new command', async () => {
      const newCommand = {
        name: 'test-deploy',
        description: 'Deploy the application',
        prompt: 'deploy the application to production',
        conditions: {
          requiresTaskNotRunning: true,
        },
        icon: 'Rocket',
        variant: 'default',
        active: true,
      }

      const response = await app.request('/api/v1/commands', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(newCommand),
      })

      expect(response.status).toBe(200)
      const created = (await response.json()) as Command

      expect(created.name).toBe('test-deploy')
      expect(created.description).toBe('Deploy the application')
      expect(created.prompt).toBe('deploy the application to production')

      // Verify it was persisted in the database
      const fromDb = await repository.get('test-deploy')
      expect(fromDb).not.toBeNull()
      expect(fromDb?.name).toBe('test-deploy')
    })

    it('should reject duplicate command names', async () => {
      const command = {
        name: 'duplicate-test',
        description: 'Test command',
        prompt: 'test',
      }

      // Create first command
      await app.request('/api/v1/commands', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(command),
      })

      // Try to create duplicate
      const response = await app.request('/api/v1/commands', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(command),
      })

      expect(response.status).toBe(400)
    })
  })

  describe('GET /api/v1/commands', () => {
    it('should list all commands', async () => {
      // Seed default commands by making first request
      await app.request('/api/v1/commands', { method: 'GET' })

      // Create additional command via API to ensure it goes through the service
      await app.request('/api/v1/commands', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: 'custom-cmd',
          description: 'Custom command',
          prompt: 'custom prompt',
        }),
      })

      const response = await app.request('/api/v1/commands', {
        method: 'GET',
      })

      expect(response.status).toBe(200)
      const body = (await response.json()) as { commands: Command[] }

      expect(body.commands.length).toBeGreaterThan(6) // At least 6 defaults + 1 custom
      const commandNames = body.commands.map((c) => c.name)
      expect(commandNames).toContain('custom-cmd')
    })
  })

  describe('GET /api/v1/commands/:commandName', () => {
    it('should get a specific command', async () => {
      await repository.save({
        name: 'get-test',
        description: 'Test command',
        prompt: 'test prompt',
      })

      const response = await app.request('/api/v1/commands/get-test', {
        method: 'GET',
      })

      expect(response.status).toBe(200)
      const command = (await response.json()) as Command

      expect(command.name).toBe('get-test')
      expect(command.description).toBe('Test command')
    })

    it('should return 404 for non-existent command', async () => {
      const response = await app.request('/api/v1/commands/non-existent', {
        method: 'GET',
      })

      expect(response.status).toBe(404)
    })
  })

  describe('PATCH /api/v1/commands/:commandName', () => {
    it('should update a command', async () => {
      // Create via API to ensure service initialization
      const createResponse = await app.request('/api/v1/commands', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: 'updatetest',
          description: 'Original description',
          prompt: 'original prompt',
        }),
      })
      expect(createResponse.status).toBe(200)

      // Verify the command was actually created
      const checkResponse = await app.request('/api/v1/commands/updatetest', {
        method: 'GET',
      })
      expect(checkResponse.status).toBe(200)

      const updates = {
        description: 'Updated description',
        prompt: 'updated prompt',
      }

      const response = await app.request('/api/v1/commands/updatetest', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updates),
      })

      expect(response.status).toBe(200)
      const updated = (await response.json()) as Command

      expect(updated.name).toBe('updatetest')
      expect(updated.description).toBe('Updated description')
      expect(updated.prompt).toBe('updated prompt')

      // Verify persistence
      const fromDb = await repository.get('updatetest')
      expect(fromDb?.description).toBe('Updated description')
    })

    it('should handle command name changes', async () => {
      // Create via API to ensure service initialization
      const createResponse = await app.request('/api/v1/commands', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: 'oldname',
          description: 'Test command',
          prompt: 'test',
        }),
      })
      expect(createResponse.status).toBe(200)

      const updates = {
        name: 'newname',
        description: 'Updated command',
      }

      const response = await app.request('/api/v1/commands/oldname', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updates),
      })

      expect(response.status).toBe(200)
      const updated = (await response.json()) as Command
      expect(updated.name).toBe('newname')

      // Verify old name is deleted and new name exists
      const oldNameExists = await repository.exists('oldname')
      const newNameExists = await repository.exists('newname')
      expect(oldNameExists).toBe(false)
      expect(newNameExists).toBe(true)
    })

    it('should return 404 for non-existent command', async () => {
      const response = await app.request('/api/v1/commands/non-existent', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ description: 'test' }),
      })

      expect(response.status).toBe(404)
    })
  })

  describe('DELETE /api/v1/commands/:commandName', () => {
    it('should delete a command', async () => {
      await repository.save({
        name: 'delete-test',
        description: 'Test command',
        prompt: 'test',
      })

      const response = await app.request('/api/v1/commands/delete-test', {
        method: 'DELETE',
      })

      expect(response.status).toBe(200)

      // Verify it was deleted from database
      const exists = await repository.exists('delete-test')
      expect(exists).toBe(false)
    })

    it('should return 404 for non-existent command', async () => {
      const response = await app.request('/api/v1/commands/non-existent', {
        method: 'DELETE',
      })

      expect(response.status).toBe(404)
    })
  })

  describe('Database Integration', () => {
    it('should persist commands across service restarts', async () => {
      // Create a command
      await repository.save({
        name: 'persist-test',
        description: 'Persistence test',
        prompt: 'test persistence',
        conditions: { requiresTaskNotRunning: true },
        icon: 'TestIcon',
        variant: 'outline',
        active: true,
      })

      // Retrieve it via API (simulating a restart)
      const response = await app.request('/api/v1/commands/persist-test', {
        method: 'GET',
      })

      expect(response.status).toBe(200)
      const command = (await response.json()) as Command

      expect(command.name).toBe('persist-test')
      expect(command.description).toBe('Persistence test')
      expect(command.conditions).toEqual({ requiresTaskNotRunning: true })
      expect(command.icon).toBe('TestIcon')
      expect(command.variant).toBe('outline')
      expect(command.active).toBe(true)
    })

    it('should handle complex conditions in JSON column', async () => {
      const complexConditions = {
        requiresUncommittedChanges: true,
        requiresCommittedChanges: false,
        requiresAheadOfMain: true,
        requiresTaskNotRunning: true,
        allowedGitSyncStatuses: ['ahead', 'diverged'],
        allowedTaskStatuses: ['open', 'error'],
      }

      await repository.save({
        name: 'complex-test',
        description: 'Complex conditions test',
        prompt: 'test',
        conditions: complexConditions,
      })

      const fromDb = await repository.get('complex-test')
      expect(fromDb?.conditions).toEqual(complexConditions)
    })
  })
})
