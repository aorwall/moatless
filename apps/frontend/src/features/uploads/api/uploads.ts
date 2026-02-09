import type { UploadResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  getDownloadFileUrl,
  uploadFile as uploadFileHandler,
} from '@moatless/api-client/sandbox/file-storage/file-storage'

export const uploadsApi = {
  async uploadFile(file: File): Promise<UploadResponse> {
    return await unwrapResponse(uploadFileHandler({ file }))
  },

  getFileUrl(fileId: string): string {
    return getDownloadFileUrl(fileId)
  },
}
