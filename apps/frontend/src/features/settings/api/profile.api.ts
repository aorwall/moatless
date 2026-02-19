import { apiClient } from '@/lib/api/client'

export interface UpdateProfileRequest {
  name?: string
  email?: string
}

export interface UpdateProfileResponse {
  user: {
    id: string
    login: string
    email: string | null
    name: string | null
    avatarUrl: string | null
    provider: string
    role: string
  } | null
  expiresAt: string | null
}

export const profileApi = {
  updateProfile: (data: UpdateProfileRequest) =>
    apiClient.patch<UpdateProfileResponse>('/api/v1/auth/me', data),
}
