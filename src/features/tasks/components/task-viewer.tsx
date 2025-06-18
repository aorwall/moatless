import { Clock, DollarSign, MessageSquare } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { TaskIteration, TaskMetadata } from '@/types/task';

interface TaskViewerProps {
  metadata: TaskMetadata;
  onIterationClick?: (iteration: TaskIteration) => void;
}

export function TaskViewer({ metadata, onIterationClick }: TaskViewerProps) {
  const formatDuration = (durationMs: number) => {
    const minutes = Math.floor(durationMs / 60000);
    const seconds = Math.floor((durationMs % 60000) / 1000);
    return `${minutes}m ${seconds}s`;
  };

  const formatCost = (cost: number) => {
    return `$${cost.toFixed(4)}`;
  };

  const handleIterationClick = (iteration: TaskIteration) => {
    onIterationClick?.(iteration);
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
    <ScrollArea className="h-full">
      <div className="p-6 space-y-4">
        {metadata.iterations.map((iteration) => (
          <div key={iteration.iteration_id} className="space-y-4">
            {/* Initial Message */}
            <div className="bg-gray-50 rounded-lg p-4">
              <div className="text-sm text-gray-900 prose prose-sm max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {iteration.initial_message}
                </ReactMarkdown>
              </div>
            </div>

            {/* Iteration Info */}
            <div className="flex justify-center py-2">
              <button
                type="button"
                className="cursor-pointer text-sm text-gray-500 hover:text-gray-700 transition-colors flex items-center gap-4 bg-transparent border-none p-0"
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
              <div className="text-sm text-gray-900 prose prose-sm max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {iteration.result}
                </ReactMarkdown>
              </div>
            )}

            {/* Error Message */}
            {iteration.error_message && (
              <div className="text-sm text-red-900 prose prose-sm max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {iteration.error_message}
                </ReactMarkdown>
              </div>
            )}
          </div>
        ))}
      </div>
    </ScrollArea>
  );
}
