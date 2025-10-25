import { Container } from 'inversify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Storage } from '../../../../infrastructure/storage/storage.js'
import { InfrastructureStorageSymbol } from '../../../../infrastructure/storage/storage.js'
import { EventsStorage } from '../events.storage.js'

describe('EventsStorage', () => {
  let container: Container
  let eventsStorage: EventsStorage
  let mockStorage: Storage

  beforeEach(() => {
    container = new Container()

    mockStorage = {
      save: vi.fn(),
      get: vi.fn(),
      delete: vi.fn(),
      exists: vi.fn(),
      list: vi.fn(),
    } as Storage

    container.bind(InfrastructureStorageSymbol).toConstantValue(mockStorage)
    container.bind(EventsStorage).toSelf()

    eventsStorage = container.get(EventsStorage)
  })

  afterEach(() => {
    container.unbindAll()
  })

  describe('saveEvents', () => {
    it('should save events with correct key structure', async () => {
      const taskId = 'task-123'
      const events: any[] = [
        {
          eventId: 'event-1',
          type: 'task' as const,
          subType: 'created' as const,
          taskId,
          timestamp: new Date(),
        },
      ]

      await eventsStorage.saveEvents(taskId, events)

      expect(mockStorage.save).toHaveBeenCalledWith('events/task-123', events)
      expect(mockStorage.save).toHaveBeenCalledTimes(1)
    })
  })

  describe('getEvents', () => {
    it('should retrieve events from storage with correct key', async () => {
      const taskId = 'task-123'
      const mockEvents: any[] = [
        {
          eventId: 'event-1',
          type: 'task' as const,
          subType: 'created' as const,
          taskId,
          timestamp: new Date(),
        },
      ]

      ;(mockStorage.get as ReturnType<typeof vi.fn>).mockResolvedValue(mockEvents)

      const result = await eventsStorage.getEvents(taskId)

      expect(result).toEqual(mockEvents)
      expect(mockStorage.get).toHaveBeenCalledWith('events/task-123')
      expect(mockStorage.get).toHaveBeenCalledTimes(1)
    })

    it('should return empty array when no events exist', async () => {
      const taskId = 'task-456'
      ;(mockStorage.get as ReturnType<typeof vi.fn>).mockResolvedValue(null)

      const result = await eventsStorage.getEvents(taskId)

      expect(result).toEqual([])
      expect(mockStorage.get).toHaveBeenCalledWith('events/task-456')
    })
  })
})
