import { createFileRoute, Outlet } from '@tanstack/react-router'

export const Route = createFileRoute('/sandbox')({
  component: SandboxLayout,
})

function SandboxLayout() {
  return <Outlet />
}
