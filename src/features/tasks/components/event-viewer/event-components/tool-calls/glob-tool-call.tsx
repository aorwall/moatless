import { FolderSearch } from 'lucide-react';
import type { ReactNode } from 'react';

interface GlobToolCallProps {
  input: {
    pattern: string;
    path?: string;
  };
  name: string;
}

export function GlobToolCall({ input, name }: GlobToolCallProps): ReactNode {
  return (
    <div className="flex items-center gap-2 flex-wrap">
      <FolderSearch className="h-3.5 w-3.5 text-teal-600 dark:text-teal-400" />
      <span className="font-medium text-foreground">{name}</span>
      <span className="text-xs text-muted-foreground">-</span>
      <code className="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">
        {input.pattern}
      </code>
      {input.path && (
        <>
          <span className="text-xs text-muted-foreground">in</span>
          <code className="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">
            {input.path}
          </code>
        </>
      )}
    </div>
  );
}
