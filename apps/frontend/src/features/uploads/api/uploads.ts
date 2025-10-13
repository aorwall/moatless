import { buildPath, type RouteParams, type RouteSuccess } from '@moatless/api-contract'
import { getUploadedFileRoute, uploadFileRoute } from '@moatless/api-contract/uploads.ts'

type UploadFileResponse = RouteSuccess<typeof uploadFileRoute>

export const uploadsApi = {
  async uploadFile(file: File): Promise<UploadFileResponse> {
    const formData = new FormData()
    formData.append('file', file)

    const path = buildPath(uploadFileRoute)
    const response = await fetch(path, {
      method: 'POST',
      body: formData,
    })

    if (!response.ok) {
      const error = await response.json()
      throw new Error(error.error || 'Failed to upload file')
    }

    return await response.json()
  },

  getFileUrl(fileId: RouteParams<typeof getUploadedFileRoute>['fileId']): string {
    return buildPath(getUploadedFileRoute, { fileId })
  },
}
