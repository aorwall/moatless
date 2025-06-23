import { ChevronDown, ChevronUp, FileEdit } from 'lucide-react';
import type { ReactNode } from 'react';
import { useMemo, useState } from 'react';

interface WriteToolCallProps {
  input: {
    file_path: string;
    content: string;
  };
  name: string;
}

const MAX_LINES = 10;

export function WriteToolCall({ input, name }: WriteToolCallProps): ReactNode {
  const [isExpanded, setIsExpanded] = useState(false);

  const { truncatedContent, lineCount, isTruncated } = useMemo(() => {
    const lines = input.content.split('\n');
    const totalLines = lines.length;

    if (!isExpanded && totalLines > MAX_LINES) {
      const truncated = lines.slice(0, MAX_LINES);
      return {
        truncatedContent: truncated.join('\n'),
        lineCount: totalLines,
        isTruncated: true,
      };
    }

    return {
      truncatedContent: input.content,
      lineCount: totalLines,
      isTruncated: false,
    };
  }, [input.content, isExpanded]);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <FileEdit className="h-3.5 w-3.5 text-green-600 dark:text-green-400" />
        <span className="font-medium text-foreground">{name}</span>
        <span className="text-xs text-muted-foreground">-</span>
        <code className="text-xs bg-muted px-1.5 py-0.5 rounded font-mono">
          {input.file_path}
        </code>
        <span className="text-xs text-muted-foreground">
          ({lineCount} lines)
        </span>
      </div>

      <div className="relative">
        <pre className="text-xs bg-muted rounded-lg p-3 overflow-x-auto font-mono">
          <code>{truncatedContent}</code>
        </pre>

        {isTruncated && (
          <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-muted to-transparent h-8 pointer-events-none" />
        )}
      </div>

      {(isTruncated || isExpanded) && (
        <button
          type="button"
          onClick={() => setIsExpanded(!isExpanded)}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          {isExpanded ? (
            <>
              <ChevronUp className="h-3 w-3" />
              Show less
            </>
          ) : (
            <>
              <ChevronDown className="h-3 w-3" />
              Show all {lineCount} lines
            </>
          )}
        </button>
      )}
    </div>
  );
}
