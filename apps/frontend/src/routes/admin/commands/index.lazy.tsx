import { createLazyFileRoute } from '@tanstack/react-router'
import { CommandsPage } from '@/features/commands'

export const Route = createLazyFileRoute('/admin/commands/')({
  component: CommandsPage,
})
