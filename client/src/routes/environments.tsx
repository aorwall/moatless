import { createFileRoute, Outlet } from '@tanstack/react-router'

export const Route = createFileRoute('/environments')({
  component: EnvironmentsLayout,
})

function EnvironmentsLayout() {
  return <Outlet />
}
