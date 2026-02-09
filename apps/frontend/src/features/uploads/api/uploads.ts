import type { UploadResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  getDownloadFileUrl,
  uploadFile as uploadFileHandler,
} from '@moatless/api-client/sandbox/file-storage/file-storage'
import type { UploadFileResponse } from '@moatless/types'

export const uploadsApi = {
  async uploadFile(file: File): Promise<UploadFileResponse> {
    const result = await unwrapResponse<UploadResponse>(uploadFileHandler({ file }))
    return result as unknown as UploadFileResponse
  },

  getFileUrl(fileId: string): string {
    return getDownloadFileUrl(fileId)
  },
}
