import { useMutation, useQueryClient } from '@tanstack/react-query'
import type { UpdateProfileRequest } from '../api/profile.api'
import { profileApi } from '../api/profile.api'

export function useUpdateProfile() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (data: UpdateProfileRequest) => profileApi.updateProfile(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['auth', 'session'] })
    },
  })
}
