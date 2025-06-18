import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { toast } from 'sonner';

import { EnvironmentForm, useCreateEnvironment } from '@/features/environments';
import { getErrorMessage } from '@/lib/utils/error';
import type { CreateEnvironmentRequest } from '@/types/environment';

export const Route = createFileRoute('/environments/new')({
  component: EnvironmentNew,
});

function EnvironmentNew() {
  const navigate = useNavigate();
  const createEnvironment = useCreateEnvironment();

  const handleSubmit = (data: CreateEnvironmentRequest) => {
    createEnvironment.mutate(data, {
      onSuccess: (createdEnvironment) => {
        toast.success('Environment created successfully');
        // Navigate to the edit page to configure code execution
        navigate({
          to: '/environments/$environmentId',
          params: { environmentId: createdEnvironment.id },
        });
      },
      onError: (error) => {
        toast.error('Failed to create environment', {
          description: getErrorMessage(error),
        });
      },
    });
  };

  return (
    <div className="container mx-auto py-6">
      <div className="max-w-2xl mx-auto">
        <EnvironmentForm
          onSubmit={handleSubmit}
          isSubmitting={createEnvironment.isPending}
        />
      </div>
    </div>
  );
}
