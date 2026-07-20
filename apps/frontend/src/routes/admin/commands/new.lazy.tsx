import { createLazyFileRoute, useNavigate } from '@tanstack/react-router'
import { CommandForm } from '@/features/commands/components/command-form'

export const Route = createLazyFileRoute('/admin/commands/new')({
  component: NewCommandPage,
})

function NewCommandPage() {
  const navigate = useNavigate()

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div className="space-y-1">
        <h1 className="text-2xl font-semibold">New command</h1>
        <p className="text-sm text-muted-foreground">Configure a quick action command for tasks.</p>
      </div>
      <CommandForm
        onSaved={(command) =>
          navigate({ to: '/admin/commands/$name', params: { name: command.name } })
        }
      />
    </div>
  )
}
