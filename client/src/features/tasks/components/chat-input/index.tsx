import { useState } from 'react';

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments';

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

  // Callbacks
  onSendMessage?: (taskId: string, message: string) => void;
  onCreateTask?: (message: string, environmentId: string) => void;
  isLoading?: boolean;
}

export function ChatInput({
  taskId,
  placeholder,
  taskStatus,
  showEnvironmentSelector = false,
  showTitle = false,
  title = 'What should we build today?',
  onSendMessage,
  onCreateTask,
  isLoading = false,
  className = '',
  ...divProps
}: ChatInputProps) {
  const [message, setMessage] = useState('');
  const [selectedEnvironmentId, setSelectedEnvironmentId] = useState<
    string | undefined
  >();

  // Environment data (only needed for homepage)
  const { data: environments, isLoading: environmentsLoading } =
    useEnvironmentsList({
      enabled: showEnvironmentSelector,
    });

  const isTaskContext = !!taskId;
  const isPending = isLoading;

  const handleSendMessage = () => {
    if (!message.trim()) {
      return;
    }

    if (isTaskContext) {
      // Task context - send message
      if (onSendMessage && !isPending) {
        onSendMessage(taskId, message.trim());
        setMessage('');
      }
    } else if (selectedEnvironmentId && onCreateTask && !isPending) {
      // Homepage context - create task
      onCreateTask(message.trim(), selectedEnvironmentId);
      setMessage('');
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

      <div className={showTitle ? 'max-w-2xl mx-auto space-y-4' : 'space-y-4'}>
        <ChatForm
          message={message}
          onMessageChange={setMessage}
          sendMessage={handleSendMessage}
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
      </div>
    </div>
  );
}
