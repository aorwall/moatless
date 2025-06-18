import { Card, CardContent } from '@/lib/components/ui/card';
import { ScrollArea } from '@/lib/components/ui/scroll-area';

interface TaskLogsProps {
  taskId: string;
}

export function TaskLogs({ taskId }: TaskLogsProps) {
  // TODO: Fetch actual logs from API
  const logs = `[2024-06-16 08:42:15] Task started: ${taskId}
[2024-06-16 08:42:16] Connecting to Claude API...
[2024-06-16 08:42:18] Analyzing codebase structure...
[2024-06-16 08:42:20] Processing user request...
[2024-06-16 08:42:22] Executing tool: Read file
[2024-06-16 08:42:24] Executing tool: Edit file
[2024-06-16 08:42:26] Changes applied successfully
[2024-06-16 08:42:28] Running validation checks...
[2024-06-16 08:42:30] Task completed successfully`;

  return (
    <ScrollArea className="h-full">
      <div className="p-6">
        <Card>
          <CardContent className="p-4">
            <pre className="text-sm font-mono whitespace-pre-wrap">{logs}</pre>
          </CardContent>
        </Card>
      </div>
    </ScrollArea>
  );
}
