import { createFileRoute, redirect } from '@tanstack/react-router'

// Triggers were renamed to Loops. Keep old links working.
export const Route = createFileRoute('/admin/triggers/$trigger-id/')({
  beforeLoad: ({ params }) => {
    throw redirect({
      to: '/admin/loops/$loop-id',
      params: { 'loop-id': params['trigger-id'] },
    })
  },
})
