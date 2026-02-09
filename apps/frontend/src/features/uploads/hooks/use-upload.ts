import type { UploadResponse } from '@moatless/api-client'
import type { UseMutationOptions } from '@tanstack/react-query'
import { useMutation } from '@tanstack/react-query'
import { uploadsApi } from '../api/uploads'

export function useUploadFile(options?: UseMutationOptions<UploadResponse, Error, File>) {
  return useMutation({
    mutationFn: (file: File) => uploadsApi.uploadFile(file),
    ...options,
  })
}
