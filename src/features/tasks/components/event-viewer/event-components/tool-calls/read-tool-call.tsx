import { FileText } from 'lucide-react';
import type { ReactNode } from 'react';

interface ReadToolCallProps {
  input: {
    file_path: string;
    offset?: number;
    limit?: number;
  };
  name: string;
}

export function ReadToolCall({ input, name }: ReadToolCallProps): ReactNode {
  return (
    <div className="flex items-center gap-2">
      <FileText className="h-3.5 w-3.5 text-purple-600 dark:text-purple-400" />
      <span className="font-medium text-foreground">{name}</span>
      <span className="text-xs text-muted-foreground">-</span>
      <code className="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">
        {input.file_path}
      </code>
      {(input.offset !== undefined || input.limit !== undefined) && (
        <span className="text-xs text-muted-foreground">
          {input.offset !== undefined && `L${input.offset}`}
          {input.offset !== undefined && input.limit !== undefined && '-'}
          {input.limit !== undefined && `L${(input.offset || 0) + input.limit}`}
        </span>
      )}
    </div>
  );
}
