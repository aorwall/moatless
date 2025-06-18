import { Wrench } from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/lib/components/ui/button';
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/lib/components/ui/card';

import type { ToolCall } from '../hooks/use-claude-code-parser';

interface ToolCallsProps {
  toolCalls: Array<ToolCall>;
}

interface ToolCallItemProps {
  toolCall: ToolCall;
}

const truncateValue = (
  value: unknown,
  maxLength: number = 100,
): { value: string; isTruncated: boolean } => {
  const stringValue =
    typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  if (stringValue.length <= maxLength) {
    return { value: stringValue, isTruncated: false };
  }
  return { value: `${stringValue.slice(0, maxLength)}...`, isTruncated: true };
};

function ToolCallItem({ toolCall }: ToolCallItemProps) {
  const [expandedParams, setExpandedParams] = useState<Set<string>>(new Set());

  const toggleParamExpansion = (paramKey: string) => {
    const newExpanded = new Set(expandedParams);
    if (newExpanded.has(paramKey)) {
      newExpanded.delete(paramKey);
    } else {
      newExpanded.add(paramKey);
    }
    setExpandedParams(newExpanded);
  };

  const hasParams = Object.keys(toolCall.input).length > 0;

  return (
    <Card className="border-l-4 border-l-blue-500">
      <CardHeader>
        <Button variant="ghost" className="w-full justify-between p-0 h-auto">
          <div className="flex items-center gap-2">
            <Wrench className="h-4 w-4 text-blue-500" />
            <CardTitle className="text-sm font-medium">
              {toolCall.name}
            </CardTitle>
          </div>
        </Button>
      </CardHeader>
      <CardContent>
        <CardContent>
          {hasParams ? (
            <div className="space-y-2">
              {Object.entries(toolCall.input).map(([key, value]) => {
                const { value: truncatedValue, isTruncated } =
                  truncateValue(value);
                const isParamExpanded = expandedParams.has(key);

                return (
                  <div key={key} className="space-y-1">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-medium text-muted-foreground">
                        {key}:
                      </span>
                      {isTruncated && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => toggleParamExpansion(key)}
                          className="h-4 px-1 text-xs"
                        >
                          {isParamExpanded ? 'less' : 'more'}
                        </Button>
                      )}
                    </div>
                    <pre className="text-xs bg-muted p-2 rounded overflow-auto max-h-32 whitespace-pre-wrap">
                      {isParamExpanded
                        ? JSON.stringify(value, null, 2)
                        : truncatedValue}
                    </pre>
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">No parameters</p>
          )}
        </CardContent>
      </CardContent>
    </Card>
  );
}

export function ToolCalls({ toolCalls }: ToolCallsProps) {
  if (!toolCalls || toolCalls.length === 0) {
    return null;
  }

  return (
    <div className="space-y-2">
      {toolCalls.map((toolCall) => (
        <ToolCallItem key={toolCall.id} toolCall={toolCall} />
      ))}
    </div>
  );
}
