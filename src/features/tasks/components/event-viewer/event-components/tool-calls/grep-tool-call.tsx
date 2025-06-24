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
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Search className="h-3.5 w-3.5 text-orange-600 dark:text-orange-400" />
        <span className="font-medium text-foreground">{name}</span>
      </div>

      <div className="bg-zinc-900 dark:bg-zinc-950 rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-green-400 select-none">$</span>
          <code className="text-zinc-100 break-all whitespace-pre-wrap">
            grep "{input.pattern}"{input.path ? ` ${input.path}` : ''}
            {input.include ? ` --include="${input.include}"` : ''}
          </code>
        </div>
      </div>
    </div>
  );
}
