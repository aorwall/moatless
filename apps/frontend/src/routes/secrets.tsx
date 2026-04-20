import { createFileRoute } from '@tanstack/react-router'
import { SecretsPanel } from '@/features/secrets/components/secrets-panel'

export const Route = createFileRoute('/secrets')({
  component: SecretsRoute,
})

function SecretsRoute() {
  return <SecretsPanel />
}
