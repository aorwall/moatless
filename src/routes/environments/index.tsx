import { createFileRoute } from '@tanstack/react-router';

import { EnvironmentList, useEnvironmentsList } from '@/features/environments';

export const Route = createFileRoute('/environments/')({
  component: EnvironmentsIndex,
});

function EnvironmentsIndex() {
  const { data: environments, isLoading } = useEnvironmentsList();

  return (
    <div className="container mx-auto py-6">
      <EnvironmentList environments={environments} isLoading={isLoading} />
    </div>
  );
}
