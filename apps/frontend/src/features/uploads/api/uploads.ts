import type { UploadFileResponse } from '@moatless/shared/types/api'

export const uploadsApi = {
  async uploadFile(file: File): Promise<UploadFileResponse> {
    const formData = new FormData()
    formData.append('file', file)

    const response = await fetch('/api/v1/uploads', {
      method: 'POST',
      body: formData,
    })

    if (!response.ok) {
      const error = await response.json()
      throw new Error(error.error || 'Failed to upload file')
    }

    return await response.json()
  },

  getFileUrl(uploadId: string, filename: string): string {
    return `/api/v1/uploads/${uploadId}/${filename}`
  },
}
