import { useParams } from '@tanstack/react-router';

import { useTask } from '@/features/tasks/hooks/use-tasks';

import { ChatInput } from './chat-input';

interface TaskChatInputProps {
  className?: string;
}

export function TaskChatInput({ className }: TaskChatInputProps) {
  const params = useParams({ from: '/tasks/$taskId' });
  const { data: metadata } = useTask(params.taskId);

  return (
    <ChatInput
      data-testid="task-chat-input"
      taskId={params.taskId}
      taskStatus={metadata?.status}
      className={className}
    />
  );
}
