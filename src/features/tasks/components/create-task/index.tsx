import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments';
import { EnvironmentSelector } from '@/features/tasks/components/chat-input/environment-selector';
import { useCreateTask } from '@/features/tasks/hooks/use-tasks';
import { Button } from '@/lib/components/ui/button';
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/lib/components/ui/card';
import { getErrorMessage } from '@/lib/utils/error';

export function CreateTask() {
  const navigate = useNavigate();
  const [selectedEnvironmentId, setSelectedEnvironmentId] =
    useState<string>('');

  const { data: environments, isLoading: environmentsLoading } =
    useEnvironmentsList();

  const createTask = useCreateTask({
    onSuccess: (data) => {
      navigate({ to: '/tasks/$taskId', params: { taskId: data.task_id } });
    },
  });

  const handleCreateTask = () => {
    if (!selectedEnvironmentId) {
      return;
    }

    createTask.mutate({
      agent_type: 'claude_code',
      message: '',
      environment_id: selectedEnvironmentId,
    });
  };

  return (
    <Card className="mx-auto max-w-2xl">
      <CardHeader>
        <CardTitle className="text-center text-2xl">Create New Task</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="space-y-2">
          <div className="text-sm font-medium text-gray-700">
            Select Environment
          </div>
          <EnvironmentSelector
            environments={environments || []}
            loading={environmentsLoading}
            selectedEnvironmentId={selectedEnvironmentId}
            onEnvironmentChange={setSelectedEnvironmentId}
          />
        </div>

        {createTask.error && (
          <div className="rounded-md bg-red-50 p-4 text-sm text-red-800">
            {getErrorMessage(createTask.error)}
          </div>
        )}

        <Button
          onClick={handleCreateTask}
          disabled={!selectedEnvironmentId || createTask.isPending}
          className="w-full"
          size="lg"
        >
          {createTask.isPending ? 'Creating...' : 'Create Task'}
        </Button>
      </CardContent>
    </Card>
  );
}
