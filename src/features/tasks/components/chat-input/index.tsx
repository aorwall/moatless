import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { toast } from 'sonner';

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments';
import {
  useCreateTask,
  useSendMessage,
} from '@/features/tasks/hooks/use-tasks';
import { getErrorMessage } from '@/lib/utils/error';

import { ChatForm } from './chat-form';
import { EnvironmentSelector } from './environment-selector';

interface ChatInputProps extends React.HTMLAttributes<HTMLDivElement> {
  // For task context
  taskId?: string;
  placeholder?: string;

  // For homepage context
  showEnvironmentSelector?: boolean;
  showTitle?: boolean;
  title?: string;

  // Task status for conditional behavior
  taskStatus?: string;
}

export function ChatInput({
  taskId,
  placeholder,
  taskStatus,
  showEnvironmentSelector = false,
  showTitle = false,
  title = 'What should we build today?',
  className = '',
  ...divProps
}: ChatInputProps) {
  const [message, setMessage] = useState('');
  const [selectedEnvironmentId, setSelectedEnvironmentId] = useState<
    string | undefined
  >();
  const navigate = useNavigate();

  // Environment data (only needed for homepage)
  const { data: environments, isLoading: environmentsLoading } =
    useEnvironmentsList({
      enabled: showEnvironmentSelector,
    });

  // Task message sending (for task context)
  const sendMessage = useSendMessage({
    onSuccess: () => {
      setMessage('');
    },
    onError: (error) => {
      console.error('Failed to send message:', error);
      toast.error('Failed to send message', {
        description: getErrorMessage(error),
      });
    },
  });

  // Task creation (for homepage context)
  const createTask = useCreateTask({
    onSuccess: (response) => {
      setMessage('');
      navigate({ to: `/tasks/${response.task_id}` });
    },
    onError: (error) => {
      console.error('Failed to create task:', error);
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      });
    },
  });

  const isTaskContext = !!taskId;
  const isPending = isTaskContext
    ? sendMessage.isPending
    : createTask.isPending;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();

    if (!message.trim()) {
      return;
    }

    if (isTaskContext) {
      // Task context - send message
      if (!sendMessage.isPending) {
        sendMessage.mutate({ taskId, message: message.trim() });
      }
    } else {
      // Homepage context - create task
      if (!(selectedEnvironmentId && !createTask.isPending)) {
        return;
      }
      createTask.mutate({
        agent_type: 'claude_code',
        message: message.trim(),
        environment_id: selectedEnvironmentId,
      });
    }
  };

  const getButtonText = () => {
    if (isTaskContext) {
      if (taskStatus === 'running') {
        return 'Task Running...';
      }
      if (taskStatus === 'completed' || taskStatus === 'failed') {
        return 'Task Completed';
      }
      return isPending ? 'Sending...' : 'Send';
    }
    return isPending ? 'Starting...' : 'Start Task';
  };

  const getPlaceholderText = () => {
    if (placeholder) {
      return placeholder;
    }
    if (isTaskContext) {
      if (taskStatus === 'running') {
        return 'Task is running... Type your next request';
      }
      if (taskStatus === 'completed' || taskStatus === 'failed') {
        return `Task is ${taskStatus}. Type a new request to continue`;
      }
      return 'Type a new request... (use @ to reference files)';
    }
    return 'Describe your coding task or feature... (use @ to reference files)';
  };

  // Always allow editing the textarea
  const isTextareaDisabled = false;

  // But disable sending based on conditions
  const isSendDisabled =
    isPending ||
    !message.trim() ||
    (isTaskContext &&
      (taskStatus === 'running' ||
        taskStatus === 'completed' ||
        taskStatus === 'failed')) ||
    !(isTaskContext || selectedEnvironmentId);

  return (
    <div className={className} {...divProps}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <form
        onSubmit={handleSubmit}
        className={showTitle ? 'max-w-2xl mx-auto space-y-4' : 'space-y-4'}
      >
        <ChatForm
          message={message}
          onMessageChange={setMessage}
          onSubmit={handleSubmit}
          textareaDisabled={isTextareaDisabled}
          sendDisabled={isSendDisabled}
          placeholder={getPlaceholderText()}
          environmentId={selectedEnvironmentId}
          taskId={taskId}
          isTaskContext={isTaskContext}
          buttonText={getButtonText()}
        />

        {showEnvironmentSelector && (
          <div className="flex flex-col sm:flex-row gap-4">
            <EnvironmentSelector
              environments={environments}
              selectedEnvironmentId={selectedEnvironmentId}
              onEnvironmentChange={setSelectedEnvironmentId}
              disabled={isPending}
              loading={environmentsLoading}
            />
          </div>
        )}
      </form>

      {isTaskContext && sendMessage.isError && (
        <p className="text-sm text-red-600 dark:text-red-400 mt-2">
          Failed to send message. Please try again.
        </p>
      )}
    </div>
  );
}
