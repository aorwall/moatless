import { createFileRoute } from '@tanstack/react-router'

import { CommandList, useCommandsList } from '@/features/commands'

export const Route = createFileRoute('/commands/')({
  component: CommandsIndex,
})

function CommandsIndex() {
  const { data: commands, isLoading } = useCommandsList()

  return (
    <div className="container mx-auto py-6">
      <CommandList commands={commands} isLoading={isLoading} />
    </div>
  )
}
