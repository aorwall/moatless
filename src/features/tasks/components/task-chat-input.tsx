import { useTaskContext } from '@/features/tasks/contexts/task-context';

import { ChatInput } from './chat-input';

interface TaskChatInputProps {
  className?: string;
}

export function TaskChatInput({ className }: TaskChatInputProps) {
  const { taskId, taskStatus } = useTaskContext();

  return (
    <ChatInput taskId={taskId} taskStatus={taskStatus} className={className} />
  );
}
