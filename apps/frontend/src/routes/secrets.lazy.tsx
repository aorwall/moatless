import { createLazyFileRoute } from '@tanstack/react-router'
import { SecretsPanel } from '@/features/secrets/components/secrets-panel'

export const Route = createLazyFileRoute('/secrets')({
  component: SecretsRoute,
})

function SecretsRoute() {
  return <SecretsPanel />
}
