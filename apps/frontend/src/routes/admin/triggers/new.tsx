import { createFileRoute, redirect } from '@tanstack/react-router'

// Triggers were renamed to Loops. Keep old links working.
export const Route = createFileRoute('/admin/triggers/new')({
  beforeLoad: () => {
    throw redirect({ to: '/admin/loops/new' })
  },
})
