import { createFileRoute, redirect } from '@tanstack/react-router'
import { authSessionQueryOptions } from '@/features/auth/hooks/use-auth'
import { queryClient } from '@/lib/api/query-client'

export const Route = createFileRoute('/admin')({
  beforeLoad: async () => {
    // An expired session 401s here; land on /login rather than an error boundary.
    const session = await queryClient.ensureQueryData(authSessionQueryOptions()).catch(() => {
      throw redirect({ to: '/login' })
    })
    if (session?.user?.role !== 'admin') {
      throw redirect({ to: '/' })
    }
  },
})
