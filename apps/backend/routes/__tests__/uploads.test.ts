import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'
import { getDataDir } from '../../utils/data-dir.js'

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
    it('should upload file to temp directory when no taskId provided', async () => {
      const testContent = 'Hello world test file'
      const testFilename = 'test.txt'

      // Create FormData for file upload
      const formData = new FormData()
      const file = new File([testContent], testFilename, { type: 'text/plain' })
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
        size: testContent.length,
        mimeType: 'text/plain',
      })
      expect(result.id).toMatch(/^temp-/)
      expect(result.path).toMatch(/^\/uploads\/temp-/)
      expect(result.uploadedAt).toBeDefined()
      expect(result.absolutePath).toBeDefined()

      // Verify file was actually saved
      const savedFile = await fs.readFile(result.absolutePath, 'utf-8')
      expect(savedFile).toBe(testContent)
    })

    it('should upload file to task directory when taskId provided', async () => {
      const testContent = 'Task-specific test file'
      const testFilename = 'task-test.txt'
      const taskId = randomUUID()

      // Create FormData for file upload
      const formData = new FormData()
      const file = new File([testContent], testFilename, { type: 'text/plain' })
      formData.append('file', file)
      formData.append('taskId', taskId)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result).toMatchObject({
        id: taskId,
        filename: testFilename,
        size: testContent.length,
        mimeType: 'text/plain',
      })
      expect(result.path).toBe(`/uploads/${taskId}/${testFilename}`)

      // Verify file was saved in task directory
      const expectedPath = path.join(testUploadDir, taskId, testFilename)
      const savedFile = await fs.readFile(expectedPath, 'utf-8')
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
      const savedFile = await fs.readFile(result.absolutePath)
      expect(new Uint8Array(savedFile)).toEqual(binaryData)
    })

    it('should sanitize filenames', async () => {
      const testContent = 'Test content'
      const unsafeFilename = '../../../malicious.txt'
      const expectedFilename = 'malicious.txt'

      const formData = new FormData()
      const file = new File([testContent], unsafeFilename, { type: 'text/plain' })
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)

      const result = (await response.json()) as any
      expect(result.filename).toBe(expectedFilename)
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
      const result = (await response.json()) as any
      expect(result.error.message).toBe('No file provided')
      expect(result.error.code).toBe('HTTP_ERROR')
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
      expect(result.mimeType).toBe('application/octet-stream')
    })

    it('should return 400 for excessively long filenames', async () => {
      const testContent = 'Test content'
      const tooLongFilename = `${'a'.repeat(256)}.txt` // 260 chars total, exceeds 255 limit

      const formData = new FormData()
      const file = new File([testContent], tooLongFilename, { type: 'text/plain' })
      formData.append('file', file)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(400)
      const result = (await response.json()) as any
      expect(result.error.message).toBe('Filename too long (max 255 characters)')
      expect(result.error.code).toBe('HTTP_ERROR')
    })
  })

  describe('GET /api/v1/uploads/:uploadId/:filename', () => {
    it('should retrieve uploaded file with correct mime type', async () => {
      const testContent = 'Test file content'
      const testFilename = 'test.txt'
      const uploadId = `temp-${randomUUID()}`

      // Set up test file
      const fileDir = path.join(testUploadDir, uploadId)
      await fs.mkdir(fileDir, { recursive: true })
      const filePath = path.join(fileDir, testFilename)
      await fs.writeFile(filePath, testContent)

      const response = await app.fetch(
        new Request(`http://localhost/api/v1/uploads/${uploadId}/${testFilename}`, {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('text/plain')

      const retrievedContent = await response.text()
      expect(retrievedContent).toBe(testContent)
    })

    it('should retrieve image files with proper headers', async () => {
      const binaryData = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
      const testFilename = 'test.png'
      const uploadId = `temp-${randomUUID()}`

      // Set up test file
      const fileDir = path.join(testUploadDir, uploadId)
      await fs.mkdir(fileDir, { recursive: true })
      const filePath = path.join(fileDir, testFilename)
      await fs.writeFile(filePath, binaryData)

      const response = await app.fetch(
        new Request(`http://localhost/api/v1/uploads/${uploadId}/${testFilename}`, {
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
      const uploadId = 'non-existent'
      const filename = 'missing.txt'

      const response = await app.fetch(
        new Request(`http://localhost/api/v1/uploads/${uploadId}/${filename}`, {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
      const result = (await response.json()) as any
      expect(result.error.message).toBe('File not found')
      expect(result.error.code).toBe('HTTP_ERROR')
    })

    it('should return 400 when uploadId is empty', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads/ /test.txt', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(400)
      const result = (await response.json()) as any
      expect(result.error.message).toBe('Upload ID and filename are required')
      expect(result.error.code).toBe('HTTP_ERROR')
    })

    it('should handle various file types with correct mime types', async () => {
      const testCases = [
        { filename: 'test.json', content: '{"test": true}', expectedType: 'application/json' },
        {
          filename: 'test.js',
          content: 'console.log("test")',
          expectedType: 'application/javascript',
        },
        { filename: 'test.md', content: '# Test', expectedType: 'text/markdown' },
        { filename: 'test.html', content: '<h1>Test</h1>', expectedType: 'text/html' },
        { filename: 'test.css', content: 'body { color: red; }', expectedType: 'text/css' },
        { filename: 'test.unknown', content: 'unknown', expectedType: 'application/octet-stream' },
      ]

      for (const testCase of testCases) {
        const uploadId = `temp-${randomUUID()}`

        // Set up test file
        const fileDir = path.join(testUploadDir, uploadId)
        await fs.mkdir(fileDir, { recursive: true })
        const filePath = path.join(fileDir, testCase.filename)
        await fs.writeFile(filePath, testCase.content)

        const response = await app.fetch(
          new Request(`http://localhost/api/v1/uploads/${uploadId}/${testCase.filename}`, {
            method: 'GET',
          }),
        )

        expect(response.status).toBe(200)
        expect(response.headers.get('content-type')).toBe(testCase.expectedType)

        const retrievedContent = await response.text()
        expect(retrievedContent).toBe(testCase.content)
      }
    })
  })

  describe('File system edge cases', () => {
    it('should handle moderately long filenames gracefully', async () => {
      const testContent = 'Test content'
      const longFilename = `${'a'.repeat(100)}.txt` // Moderately long but valid filename

      const formData = new FormData()
      const file = new File([testContent], longFilename, { type: 'text/plain' })
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

    it('should create nested directories correctly', async () => {
      const testContent = 'Test content'
      const testFilename = 'test.txt'
      const taskId = 'nested/task/id'

      const formData = new FormData()
      const file = new File([testContent], testFilename, { type: 'text/plain' })
      formData.append('file', file)
      formData.append('taskId', taskId)

      const response = await app.fetch(
        new Request('http://localhost/api/v1/uploads', {
          method: 'POST',
          body: formData,
        }),
      )

      expect(response.status).toBe(200)
      const result = (await response.json()) as any
      expect(result.id).toBe(taskId)
    })
  })
})
