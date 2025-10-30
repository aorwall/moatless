import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ToolService } from '../tool.service'

describe('ToolService - Screenshot', () => {
  let toolService: ToolService
  let mockCaptureSimpleScreenshot: ReturnType<typeof vi.fn>
  let mockGetServerStatus: ReturnType<typeof vi.fn>

  beforeEach(() => {
    // Create mock functions
    mockCaptureSimpleScreenshot = vi.fn()
    mockGetServerStatus = vi.fn()

    // Create mock preview manager
    const mockPreviewManager = {
      getStatus: mockGetServerStatus,
    } as any

    // Create mock screenshot service
    const mockScreenshotService = {
      captureSimpleScreenshot: mockCaptureSimpleScreenshot,
    } as any

    // Create tool service instance with new architecture dependencies
    toolService = new ToolService(
      {} as any, // taskService
      {} as any, // runtimeManager
      mockPreviewManager, // previewManager
      {} as any, // driver
      {} as any, // repositoryService
      mockScreenshotService, // screenshotService
    )
  })

  describe('handleTakePreviewScreenshot', () => {
    const taskId = 'test-task-123'
    const serverUrl = 'http://localhost:3000'

    it('should capture screenshot without path (root URL)', async () => {
      mockGetServerStatus.mockResolvedValue({
        status: 'started',
        url: serverUrl,
      })
      mockCaptureSimpleScreenshot.mockResolvedValue('/screenshots/screenshot-123.png')

      const result = await toolService.handleTakePreviewScreenshot(taskId)

      expect(mockGetServerStatus).toHaveBeenCalledWith(taskId)
      expect(mockCaptureSimpleScreenshot).toHaveBeenCalledWith(serverUrl, undefined)
      expect(result).toEqual({
        content: [
          {
            type: 'text',
            text: 'Screenshot captured successfully. Path: /screenshots/screenshot-123.png',
          },
        ],
      })
    })

    it('should capture screenshot with relative path', async () => {
      mockGetServerStatus.mockResolvedValue({
        status: 'started',
        url: serverUrl,
      })
      mockCaptureSimpleScreenshot.mockResolvedValue('/screenshots/screenshot-123.png')

      const result = await toolService.handleTakePreviewScreenshot(taskId, { path: '/about' })

      expect(mockCaptureSimpleScreenshot).toHaveBeenCalledWith(serverUrl, '/about')
      expect(result).toEqual({
        content: [
          {
            type: 'text',
            text: 'Screenshot captured successfully at path: /about. Path: /screenshots/screenshot-123.png',
          },
        ],
      })
    })

    it('should capture screenshot with path without leading slash', async () => {
      mockGetServerStatus.mockResolvedValue({
        status: 'started',
        url: serverUrl,
      })
      mockCaptureSimpleScreenshot.mockResolvedValue('/screenshots/screenshot-123.png')

      const result = await toolService.handleTakePreviewScreenshot(taskId, { path: 'dashboard' })

      expect(mockCaptureSimpleScreenshot).toHaveBeenCalledWith(serverUrl, 'dashboard')
      expect(result).toEqual({
        content: [
          {
            type: 'text',
            text: 'Screenshot captured successfully at path: dashboard. Path: /screenshots/screenshot-123.png',
          },
        ],
      })
    })

    it('should return error if server is not started', async () => {
      mockGetServerStatus.mockResolvedValue({
        status: 'stopped',
        url: null,
      })

      const result = await toolService.handleTakePreviewScreenshot(taskId, { path: '/about' })

      expect(mockCaptureSimpleScreenshot).not.toHaveBeenCalled()
      expect(result).toEqual({
        content: [
          {
            type: 'text',
            text: 'Error: Preview server is not running. Status: stopped',
          },
        ],
        isError: true,
      })
    })

    it('should return error if server URL is missing', async () => {
      mockGetServerStatus.mockResolvedValue({
        status: 'started',
        url: null,
      })

      const result = await toolService.handleTakePreviewScreenshot(taskId)

      expect(mockCaptureSimpleScreenshot).not.toHaveBeenCalled()
      expect(result).toEqual({
        content: [
          {
            type: 'text',
            text: 'Error: Preview server is not running. Status: started',
          },
        ],
        isError: true,
      })
    })

    it('should handle screenshot service errors', async () => {
      mockGetServerStatus.mockResolvedValue({
        status: 'started',
        url: serverUrl,
      })
      mockCaptureSimpleScreenshot.mockRejectedValue(new Error('Screenshot failed'))

      const result = await toolService.handleTakePreviewScreenshot(taskId, { path: '/about' })

      expect(result).toEqual({
        content: [
          {
            type: 'text',
            text: 'Failed to capture screenshot: Screenshot failed',
          },
        ],
        isError: true,
      })
    })
  })
})
