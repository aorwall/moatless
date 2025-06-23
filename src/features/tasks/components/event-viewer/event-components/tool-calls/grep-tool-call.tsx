import { Search } from 'lucide-react';
import type { ReactNode } from 'react';

interface GrepToolCallProps {
  input: {
    pattern: string;
    path?: string;
    include?: string;
  };
  name: string;
}

export function GrepToolCall({ input, name }: GrepToolCallProps): ReactNode {
  return (
    <div className="flex items-center gap-2 flex-wrap">
      <Search className="h-3.5 w-3.5 text-orange-600 dark:text-orange-400" />
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
      {input.include && (
        <span className="text-xs text-muted-foreground">
          (include: {input.include})
        </span>
      )}
    </div>
  );
}
