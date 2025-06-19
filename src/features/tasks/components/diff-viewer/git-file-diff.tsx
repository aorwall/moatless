import { ChevronDown, ChevronRight } from 'lucide-react';
import { useState } from 'react';

import { Badge } from '@/lib/components/ui/badge';
import { Card, CardContent, CardHeader } from '@/lib/components/ui/card';
import { cn } from '@/lib/utils';
import type { GitDiffFile } from '@/types/git-diff';

import { SyntaxDiffViewer } from './syntax-diff-viewer';

interface GitFileDiffProps {
  file: GitDiffFile;
}

const statusConfig = {
  added: { label: 'Added', className: 'bg-green-100 text-green-800' },
  modified: { label: 'Modified', className: 'bg-yellow-100 text-yellow-800' },
  deleted: { label: 'Deleted', className: 'bg-red-100 text-red-800' },
  renamed: { label: 'Renamed', className: 'bg-blue-100 text-blue-800' },
} as const;

export function GitFileDiff({ file }: GitFileDiffProps) {
  const [isExpanded, setIsExpanded] = useState(true);
  const status = statusConfig[file.status];

  return (
    <Card className="w-full">
      <CardHeader
        className="cursor-pointer py-3"
        onClick={() => setIsExpanded(!isExpanded)}
      >
        <div className="flex items-center justify-between w-full">
          <div className="flex items-center gap-3 min-w-0 flex-1">
            {isExpanded ? (
              <ChevronDown className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            ) : (
              <ChevronRight className="h-4 w-4 text-muted-foreground flex-shrink-0" />
            )}
            <code className="text-sm font-medium truncate">
              {file.status === 'renamed' && file.old_path
                ? `${file.old_path} → ${file.path}`
                : file.path}
            </code>
            <Badge
              variant="secondary"
              className={cn('text-xs flex-shrink-0', status.className)}
            >
              {status.label}
            </Badge>
          </div>
          <div className="flex items-center gap-2 text-xs flex-shrink-0">
            <span className="text-green-600">+{file.additions}</span>
            <span className="text-red-600">-{file.deletions}</span>
          </div>
        </div>
      </CardHeader>
      {isExpanded && (
        <CardContent className="p-0 w-full">
          <div className="diff-container w-full overflow-x-auto">
            {file.chunks.map((chunk, chunkIndex) => (
              <div
                key={`${file.path}-chunk-${chunkIndex}`}
                className="diff-chunk w-full"
              >
                <div className="diff-header bg-gray-50 dark:bg-gray-800 px-4 py-2 text-xs text-muted-foreground font-mono w-full">
                  {chunk.header}
                </div>
                <SyntaxDiffViewer chunk={chunk} filePath={file.path} />
              </div>
            ))}
          </div>
        </CardContent>
      )}
    </Card>
  );
}
