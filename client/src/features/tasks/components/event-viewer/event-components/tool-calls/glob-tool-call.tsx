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
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FolderSearch className="h-3.5 w-3.5 text-teal-600 dark:text-teal-400" />
        <span className="font-medium text-foreground">{name}</span>
      </div>

      <div className="bg-zinc-900 dark:bg-zinc-950 rounded-lg p-3 font-mono text-sm">
        <div className="flex items-start gap-2">
          <span className="text-green-400 select-none">$</span>
          <code className="text-zinc-100 break-all whitespace-pre-wrap">
            find {input.path || '.'} -name "{input.pattern}"
          </code>
        </div>
      </div>
    </div>
  );
}
