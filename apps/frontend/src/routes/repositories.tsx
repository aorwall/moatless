import { createFileRoute, Outlet } from '@tanstack/react-router'

export const Route = createFileRoute('/repositories')({
  component: RepositoriesLayout,
})

function RepositoriesLayout() {
  return <Outlet />
}
