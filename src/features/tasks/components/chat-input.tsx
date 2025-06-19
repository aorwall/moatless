import { Send } from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/lib/components/ui/button';
import { Textarea } from '@/lib/components/ui/textarea';

import { useSendMessage } from '../hooks/use-tasks';

interface ChatInputProps {
  taskId: string;
  disabled?: boolean;
  placeholder?: string;
}

export function ChatInput({
  taskId,
  disabled = false,
  placeholder = 'Type a new request...',
}: ChatInputProps) {
  const [message, setMessage] = useState('');
  const sendMessage = useSendMessage({
    onSuccess: () => {
      setMessage('');
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (message.trim() && !sendMessage.isPending) {
      sendMessage.mutate({ taskId, message: message.trim() });
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit(e);
    }
  };

  const isDisabled = disabled || sendMessage.isPending || !message.trim();

  return (
    <form
      onSubmit={handleSubmit}
      className="p-4 border-t border-slate-200 dark:border-slate-700"
    >
      <div className="flex gap-2">
        <Textarea
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          disabled={disabled || sendMessage.isPending}
          className="min-h-[80px] resize-none flex-1"
          rows={3}
        />
        <Button
          type="submit"
          disabled={isDisabled}
          size="icon"
          className="self-end"
        >
          <Send className="h-4 w-4" />
          <span className="sr-only">Send message</span>
        </Button>
      </div>
      {sendMessage.isError && (
        <p className="text-sm text-red-600 dark:text-red-400 mt-2">
          Failed to send message. Please try again.
        </p>
      )}
    </form>
  );
}
