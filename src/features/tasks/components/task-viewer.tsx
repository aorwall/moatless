import { useParams } from '@tanstack/react-router';
import { Clock, DollarSign, Loader2, MessageSquare } from 'lucide-react';
import { useEffect, useRef } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { useTask } from '@/features/tasks/hooks/use-tasks';
import { useTaskUIStore } from '@/features/tasks/stores/task-ui-store';
import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { TaskIteration } from '@/types/task';

export function TaskViewer() {
  const params = useParams({ from: '/tasks/$taskId' });
  const { data: metadata } = useTask(params.taskId);
  const setSelectedIteration = useTaskUIStore(
    (state) => state.setSelectedIteration,
  );
  const scrollAreaRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Scroll to bottom when component mounts (initial load)
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [params.taskId]); // Trigger when taskId changes

  useEffect(() => {
    // Scroll to bottom when task is running
    if (bottomRef.current && metadata?.is_running) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  });

  if (!metadata) {
    return null;
  }
  const formatDuration = (durationMs: number) => {
    const minutes = Math.floor(durationMs / 60000);
    const seconds = Math.floor((durationMs % 60000) / 1000);
    return `${minutes}m ${seconds}s`;
  };

  const formatCost = (cost: number) => {
    return `$${cost.toFixed(4)}`;
  };

  const handleIterationClick = (iteration: TaskIteration) => {
    setSelectedIteration(iteration);
  };

  const handleKeyDown = (
    event: React.KeyboardEvent,
    iteration: TaskIteration,
  ) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      handleIterationClick(iteration);
    }
  };

  return (
    <ScrollArea className="h-full w-full" ref={scrollAreaRef}>
      <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
        {metadata.iterations?.map((iteration, index) => (
          <div
            key={iteration.iteration_id || `iteration-${index}`}
            className="space-y-4"
          >
            {/* Initial Message */}
            <div className="bg-secondary rounded-lg p-4">
              <div className="text-sm text-gray-900 dark:text-gray-100 prose prose-sm dark:prose-invert max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {iteration.initial_message}
                </ReactMarkdown>
              </div>
            </div>

            {/* Iteration Info */}
            <div className="flex justify-center py-2">
              <button
                type="button"
                className="cursor-pointer text-sm text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 transition-colors flex items-center gap-4 bg-transparent border-none p-0"
                onClick={() => handleIterationClick(iteration)}
                onKeyDown={(e) => handleKeyDown(e, iteration)}
              >
                <div className="flex items-center gap-1">
                  <Clock className="h-4 w-4" />
                  <span>{formatDuration(iteration.duration_ms)}</span>
                </div>
                <div className="flex items-center gap-1">
                  <MessageSquare className="h-4 w-4" />
                  <span>{iteration.message_count} messages</span>
                </div>
                <div className="flex items-center gap-1">
                  <DollarSign className="h-4 w-4" />
                  <span>{formatCost(iteration.cost_usd)}</span>
                </div>
                <span className="text-xs">• Click to view events</span>
              </button>
            </div>

            {/* Result Message */}
            {iteration.result && (
              <div className="text-sm text-gray-900 dark:text-gray-100 prose prose-sm dark:prose-invert max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {iteration.result}
                </ReactMarkdown>
              </div>
            )}

            {/* Error Message */}
            {iteration.error_message && (
              <div className="text-sm text-red-900 dark:text-red-400 prose prose-sm dark:prose-invert max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {iteration.error_message}
                </ReactMarkdown>
              </div>
            )}
          </div>
        ))}

        {/* Show running state after the last iteration */}
        {metadata.is_running &&
          metadata.iterations &&
          metadata.iterations.length > 0 && (
            <div className="flex items-center justify-center py-4 space-x-2">
              <Loader2 className="h-5 w-5 animate-spin text-blue-500" />
              <span className="text-sm text-gray-600 dark:text-gray-400">
                Processing...
              </span>
            </div>
          )}

        <div ref={bottomRef} />
      </div>
    </ScrollArea>
  );
}
