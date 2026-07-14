import { createLazyFileRoute } from '@tanstack/react-router'
import { SecretsPanel } from '@/features/secrets/components/secrets-panel'

export const Route = createLazyFileRoute('/admin/secrets/')({
  component: AdminSecretsPage,
})

function AdminSecretsPage() {
  // Admin section: land on global (org-wide) secrets by default.
  return <SecretsPanel defaultScope="global" />
}
