import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { toast } from 'sonner';

import { EnvironmentForm } from '@/features/settings/components/environment-form';
import { EnvironmentList } from '@/features/settings/components/environment-list';
import { useEnvironmentsSettings } from '@/features/settings/hooks/use-environments-settings';
import { getErrorMessage } from '@/lib/utils/error';
import type {
  CreateEnvironmentRequest,
  EnvironmentResponse,
  UpdateEnvironmentRequest,
} from '@/types/environment';

export const Route = createFileRoute('/settings/environments')({
  component: EnvironmentsSettings,
});

type ViewMode = 'list' | 'create' | 'edit';

function EnvironmentsSettings() {
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [editingEnvironment, setEditingEnvironment] =
    useState<EnvironmentResponse | null>(null);

  const {
    environments,
    isLoading,
    error,
    createEnvironment,
    updateEnvironment,
    deleteEnvironment,
    isCreating,
  } = useEnvironmentsSettings();

  const handleCreateNew = () => {
    setEditingEnvironment(null);
    setViewMode('create');
  };

  const handleEdit = (environment: EnvironmentResponse) => {
    setEditingEnvironment(environment);
    setViewMode('edit');
  };

  const handleCancel = () => {
    setEditingEnvironment(null);
    setViewMode('list');
  };

  const handleSubmit = (
    data: CreateEnvironmentRequest | UpdateEnvironmentRequest,
  ) => {
    if (viewMode === 'create') {
      createEnvironment(data as CreateEnvironmentRequest, {
        onSuccess: () => {
          setViewMode('list');
          toast.success('Environment created successfully');
        },
        onError: (error) => {
          console.error('Failed to create environment:', error);
          toast.error('Failed to create environment', {
            description: getErrorMessage(error),
          });
        },
      });
    } else if (viewMode === 'edit' && editingEnvironment) {
      updateEnvironment(
        { id: editingEnvironment.id, data: data as UpdateEnvironmentRequest },
        {
          onSuccess: () => {
            setViewMode('list');
            setEditingEnvironment(null);
            toast.success('Environment updated successfully');
          },
          onError: (error) => {
            console.error('Failed to update environment:', error);
            toast.error('Failed to update environment', {
              description: getErrorMessage(error),
            });
          },
        },
      );
    }
  };

  const handleDelete = (environmentId: string) => {
    deleteEnvironment(environmentId, {
      onSuccess: () => {
        toast.success('Environment deleted successfully');
      },
      onError: (error) => {
        console.error('Failed to delete environment:', error);
        toast.error('Failed to delete environment', {
          description: getErrorMessage(error),
        });
      },
    });
  };

  const handleToggleActive = (environmentId: string, isActive: boolean) => {
    updateEnvironment(
      { id: environmentId, data: { is_active: isActive } },
      {
        onSuccess: () => {
          toast.success(
            `Environment ${isActive ? 'activated' : 'deactivated'} successfully`,
          );
        },
        onError: (error) => {
          console.error('Failed to update environment:', error);
          toast.error('Failed to update environment', {
            description: getErrorMessage(error),
          });
        },
      },
    );
  };

  if (error) {
    return (
      <div className="space-y-6">
        <div className="text-center py-12">
          <div className="text-destructive">
            <p className="text-lg mb-2">Error loading environments</p>
            <p className="text-sm">{error.message}</p>
          </div>
        </div>
      </div>
    );
  }

  if (viewMode === 'create') {
    return (
      <div className="space-y-6">
        <EnvironmentForm
          onSubmit={handleSubmit}
          onCancel={handleCancel}
          isSubmitting={isCreating}
        />
      </div>
    );
  }

  if (viewMode === 'edit' && editingEnvironment) {
    return (
      <div className="space-y-6">
        <EnvironmentForm
          onSubmit={handleSubmit}
          onCancel={handleCancel}
          isSubmitting={isCreating}
          initialData={editingEnvironment}
          mode="edit"
        />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <EnvironmentList
        environments={environments}
        onCreateNew={handleCreateNew}
        onEdit={handleEdit}
        onDelete={handleDelete}
        onToggleActive={handleToggleActive}
        isLoading={isLoading}
      />
    </div>
  );
}
