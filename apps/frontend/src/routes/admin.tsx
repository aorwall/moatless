import { createFileRoute, redirect } from '@tanstack/react-router'
import { authSessionQueryOptions } from '@/features/auth/hooks/use-auth'
import { queryClient } from '@/lib/api/query-client'

export const Route = createFileRoute('/admin')({
  beforeLoad: async () => {
    const session = await queryClient.ensureQueryData(authSessionQueryOptions())
    if (session?.user?.role !== 'admin') {
      throw redirect({ to: '/' })
    }
  },
})
