import { ChevronDown, ChevronRight } from 'lucide-react';
import { useState } from 'react';

import { Button } from '@/lib/components/ui/button';
import { Markdown } from '@/lib/components/ui/markdown';

import { truncateText } from '../../hooks/use-claude-code-parser';

interface MessageContentProps {
  content: string;
  maxLength?: number;
}

export function MessageContent({
  content,
  maxLength = 200,
}: MessageContentProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const { text: truncatedText, isTruncated } = truncateText(content, maxLength);

  if (!isTruncated) {
    return <Markdown content={content} className="text-sm" />;
  }

  return (
    <div className="space-y-2">
      <Markdown
        content={isExpanded ? content : truncatedText}
        className="text-sm"
      />
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setIsExpanded(!isExpanded)}
        className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
      >
        {isExpanded ? (
          <>
            <ChevronDown className="h-3 w-3 mr-1" />
            Show less
          </>
        ) : (
          <>
            <ChevronRight className="h-3 w-3 mr-1" />
            Show more
          </>
        )}
      </Button>
    </div>
  );
}
