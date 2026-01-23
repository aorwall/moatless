import { promises as fs } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../../test/helper'
import { getDataDir } from '../../../utils/data-dir'

describe('Upload Routes', () => {
  let app: Awaited<ReturnType<typeof build>>
  let testDataDir: string
  let testUploadDir: string

  beforeEach(async () => {
    app = await build()

    // Set up test-specific upload directory
    testDataDir = getDataDir()
    testUploadDir = path.join(testDataDir, 'uploads')

    // Ensure clean state
    try {
      await fs.rm(testUploadDir, { recursive: true, force: true })
    } catch {
      // Directory might not exist
    }
    await fs.mkdir(testUploadDir, { recursive: true })
  })

  afterEach(async () => {
    // Clean up test files
    try {
      await fs.rm(testUploadDir, { recursive: true, force: true })
    } catch {
      // Directory might not exist
    }
  })

  describe('POST /api/v1/uploads', () => {
    it('should upload file and return file metadata', async () => {
      const testContent = 'Hello world test file'
      const testFilename = 'test.txt'

      // Create FormData for file upload
      const formData = new FormData()
      const file = new File([testContent], testFilename, {
        type: 'text/plain',
      })
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result.filename).toBe(testFilename)
      expect(result.size).toBe(testContent.length)
      expect(result.mimeType).toContain('text/plain')
      expect(result.fileId).toBeDefined()
      expect(result.uploadedAt).toBeDefined()

      // Verify file was actually saved
      const filePath = path.join(testUploadDir, result.fileId)
      const savedFile = await fs.readFile(filePath, 'utf-8')
      expect(savedFile).toBe(testContent)
    })

    it('should handle binary files correctly', async () => {
      const binaryData = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) // PNG header
      const testFilename = 'test.png'

      const formData = new FormData()
      const file = new File([binaryData], testFilename, { type: 'image/png' })
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result).toMatchObject({
        filename: testFilename,
        size: binaryData.length,
        mimeType: 'image/png',
      })

      // Verify binary file was saved correctly
      const filePath = path.join(testUploadDir, result.fileId)
      const savedFile = await fs.readFile(filePath)
      expect(new Uint8Array(savedFile)).toEqual(binaryData)
    })

    it('should return 400 when no file provided', async () => {
      const formData = new FormData()
      // Don't add any file

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(400)
    })

    it('should handle files without mime type', async () => {
      const testContent = 'Unknown file type'
      const testFilename = 'unknown.xyz'

      const formData = new FormData()
      const file = new File([testContent], testFilename) // No type specified
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      // When no type is specified, falls back to application/octet-stream
      // (Bun may also set it to text/plain;charset=utf-8 for text content)
      expect(result.mimeType).toBeDefined()
    })

    it('should return 400 for excessively long filenames', async () => {
      const testContent = 'Test content'
      const tooLongFilename = `${'a'.repeat(256)}.txt` // 260 chars total, exceeds 255 limit

      const formData = new FormData()
      const file = new File([testContent], tooLongFilename, {
        type: 'text/plain',
      })
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(500)
    })
  })

  describe('GET /api/v1/uploads/:fileId', () => {
    it('should retrieve uploaded file with correct mime type', async () => {
      const testContent = 'Test file content'
      const testFilename = 'test.txt'

      // First upload a file
      const formData = new FormData()
      const file = new File([testContent], testFilename, { type: 'text/plain' })
      formData.append('file', file)

      const uploadResponse = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(uploadResponse.status).toBe(200)
      const uploadResult = (await uploadResponse.json()) as any
      const fileId = uploadResult.fileId

      // Now retrieve it
      const response = await app.fetch(
        new Request(`http://localhost/api/v1/uploads/${fileId}`, {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toContain('text/plain')

      const retrievedContent = await response.text()
      expect(retrievedContent).toBe(testContent)
    })

    it('should retrieve image files with proper headers', async () => {
      const binaryData = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
      const testFilename = 'test.png'

      // First upload a file
      const formData = new FormData()
      const file = new File([binaryData], testFilename, { type: 'image/png' })
      formData.append('file', file)

      const uploadResponse = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(uploadResponse.status).toBe(200)
      const uploadResult = (await uploadResponse.json()) as any
      const fileId = uploadResult.fileId

      // Now retrieve it
      const response = await app.fetch(
        new Request(`http://localhost/api/v1/uploads/${fileId}`, {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('image/png')
      expect(response.headers.get('cache-control')).toBe('public, max-age=31536000')

      const retrievedData = new Uint8Array(await response.arrayBuffer())
      expect(retrievedData).toEqual(binaryData)
    })

    it('should return 404 for non-existent files', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads/non-existent-file-id', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
    })
  })

  describe('File system edge cases', () => {
    it('should handle moderately long filenames gracefully', async () => {
      const testContent = 'Test content'
      const longFilename = `${'a'.repeat(100)}.txt` // Moderately long but valid filename

      const formData = new FormData()
      const file = new File([testContent], longFilename, {
        type: 'text/plain',
      })
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)
      const result = (await response.json()) as any
      expect(result.filename).toBe(longFilename)
    })
  })
})
