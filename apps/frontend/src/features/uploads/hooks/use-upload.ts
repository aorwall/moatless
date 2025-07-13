import type { UploadFileResponse } from '@moatless/types/types/api'
import type { UseMutationOptions } from '@tanstack/react-query'
import { useMutation } from '@tanstack/react-query'
import { uploadsApi } from '../api/uploads'

export function useUploadFile(options?: UseMutationOptions<UploadFileResponse, Error, File>) {
  return useMutation({
    mutationFn: (file: File) => uploadsApi.uploadFile(file),
    ...options,
  })
}
