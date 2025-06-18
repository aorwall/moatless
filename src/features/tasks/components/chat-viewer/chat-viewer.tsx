import { MessageSquare } from 'lucide-react';

interface ChatViewerProps {
  taskId: string;
}

export function ChatViewer({ taskId }: ChatViewerProps) {
  // TODO: Implement chat viewer
  return (
    <div className="h-full flex items-center justify-center p-6">
      <div className="text-center space-y-3">
        <MessageSquare className="h-8 w-8 mx-auto text-muted-foreground/50" />
        <p className="text-muted-foreground">Chat interface coming soon</p>
        <p className="text-sm text-muted-foreground">Task: {taskId}</p>
      </div>
    </div>
  );
}
