import { promises as fs } from 'node:fs'
import path from 'node:path'
import { SANDBOX_DOWNLOADS_DIR } from '@moatless/sandbox-schemas'
import { z } from 'zod/v4'
import type { SandboxConfig } from '../../config/environment'
import type { BackendApiClient } from '../../shared/backend-api-client'
import { logger } from '../../shared/logger'
import type { SandboxTool } from './types'

const toolLogger = logger.child({ service: 'tool-download-file' })

/**
 * Create the downloadFile tool.
 *
 * Downloads a file attached to a user message via the backend API.
 * Files are stored in /opt/moatless/downloads/{filename}.
 *
 * This replaces the previous approach of mounting uploads via CSI driver.
 */
export function createDownloadFileTool(
  _config: SandboxConfig,
  backendApi: BackendApiClient | undefined,
): SandboxTool {
  return {
    name: 'downloadFile',
    description:
      'Download a file attached to a user message. Returns the local file path after download. Use this to access files referenced in the conversation.',
    schema: {
      fileId: z.string().describe('The file ID from the message attachment'),
      targetPath: z
        .string()
        .optional()
        .describe('Optional target path. Defaults to /opt/moatless/downloads/{filename}'),
    },
    execute: async (args) => {
      const fileId = args.fileId as string

      if (!fileId) {
        return {
          text: 'fileId is required',
          isError: true,
        }
      }

      toolLogger.info({ fileId }, 'downloadFile called')

      if (!backendApi) {
        toolLogger.warn('Backend API client not configured, cannot download file')
        return {
          text: 'Backend API not available. File download not possible.',
          isError: true,
        }
      }

      try {
        // Fetch the file from backend API
        const response = await backendApi.get(`/api/v1/uploads/${fileId}`)

        if (!response.ok) {
          const errorText = await response.text()
          toolLogger.warn(
            { fileId, status: response.status, error: errorText },
            'Failed to download file from backend API',
          )
          return {
            text: `Failed to download file: ${response.status} ${response.statusText}`,
            isError: true,
          }
        }

        // Extract filename from Content-Disposition header or use fileId
        const contentDisposition = response.headers.get('Content-Disposition')
        let filename = fileId
        if (contentDisposition) {
          const match = contentDisposition.match(/filename="?([^"]+)"?/)
          if (match?.[1]) {
            filename = match[1]
          }
        }

        // Determine save path
        const targetPath = args.targetPath as string | undefined
        const savePath = targetPath || path.join(SANDBOX_DOWNLOADS_DIR, filename)

        // Ensure directory exists
        await fs.mkdir(path.dirname(savePath), { recursive: true })

        // Write file content
        const buffer = Buffer.from(await response.arrayBuffer())
        await fs.writeFile(savePath, buffer)

        toolLogger.info({ fileId, savePath, size: buffer.length }, 'File downloaded successfully')

        return {
          text: `File downloaded successfully to: ${savePath}`,
        }
      } catch (error) {
        toolLogger.error({ err: error, fileId }, 'Error downloading file')
        return {
          text: `Failed to download file: ${error instanceof Error ? error.message : 'Unknown error'}`,
          isError: true,
        }
      }
    },
  }
}
