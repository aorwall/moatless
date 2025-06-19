import { CheckCircle, Terminal, XCircle } from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/lib/components/ui/button';
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/lib/components/ui/card';

import type { ToolResult } from '../../hooks/use-claude-code-parser';

interface ToolResultsProps {
  toolResults: Array<ToolResult>;
}

interface ToolResultItemProps {
  toolResult: ToolResult;
}

const truncateContent = (
  content: string | Record<string, unknown>,
  maxLength: number = 300,
): { content: string; isTruncated: boolean } => {
  const stringContent =
    typeof content === 'string' ? content : JSON.stringify(content, null, 2);
  if (stringContent.length <= maxLength) {
    return { content: stringContent, isTruncated: false };
  }
  return {
    content: `${stringContent.slice(0, maxLength)}...`,
    isTruncated: true,
  };
};

function ToolResultItem({ toolResult }: ToolResultItemProps) {
  const [isContentExpanded, setIsContentExpanded] = useState(false);

  const { content: truncatedContent, isTruncated } = truncateContent(
    toolResult.content,
  );
  const fullContent =
    typeof toolResult.content === 'string'
      ? toolResult.content
      : JSON.stringify(toolResult.content, null, 2);

  const isError = toolResult.is_error;

  return (
    <Card
      className={`border-l-4 ${isError ? 'border-l-red-400 dark:border-l-red-600' : 'border-l-slate-400 dark:border-l-slate-600'}`}
    >
      <CardHeader className="pb-2">
        <div className="flex items-center gap-2">
          {isError ? (
            <XCircle className="h-4 w-4 text-red-600 dark:text-red-400" />
          ) : (
            <CheckCircle className="h-4 w-4 text-slate-600 dark:text-slate-400" />
          )}
          <CardTitle className="text-sm font-medium">Tool Result</CardTitle>
        </div>
      </CardHeader>
      <CardContent className="pt-0 space-y-2">
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <Terminal className="h-3 w-3 text-muted-foreground" />
            <span className="text-xs font-medium text-muted-foreground">
              Output:
            </span>
            {isTruncated && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setIsContentExpanded(!isContentExpanded)}
                className="h-4 px-1 text-xs"
              >
                {isContentExpanded ? 'less' : 'more'}
              </Button>
            )}
          </div>
          <pre
            className={`text-xs bg-muted p-3 rounded overflow-auto whitespace-pre-wrap ${
              isError
                ? 'bg-red-50 dark:bg-red-900/10 text-red-900 dark:text-red-100'
                : ''
            } max-h-64`}
          >
            {isContentExpanded ? fullContent : truncatedContent}
          </pre>
        </div>
      </CardContent>
    </Card>
  );
}

export function ToolResults({ toolResults }: ToolResultsProps) {
  if (!toolResults || toolResults.length === 0) {
    return null;
  }

  return (
    <div className="space-y-2">
      {toolResults.map((toolResult, index) => (
        <ToolResultItem
          key={`${toolResult.tool_use_id}-${index}`}
          toolResult={toolResult}
        />
      ))}
    </div>
  );
}
