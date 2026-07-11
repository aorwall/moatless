import { createLazyFileRoute, Outlet } from '@tanstack/react-router'

export const Route = createLazyFileRoute('/sandbox')({
  component: SandboxLayout,
})

function SandboxLayout() {
  return <Outlet />
}
