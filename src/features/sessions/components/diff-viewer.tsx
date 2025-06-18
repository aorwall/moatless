import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { GitDiffResponse } from '@/types/git-diff';

import { GitFileDiff } from './git-file-diff';

interface DiffViewerProps {
  gitDiff?: GitDiffResponse;
}

export function DiffViewer({ gitDiff }: DiffViewerProps) {
  if (!gitDiff || gitDiff.files.length === 0) {
    return (
      <div className="h-full flex items-center justify-center text-muted-foreground">
        No changes to display
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-6 space-y-4">
        <div className="flex items-center justify-between text-sm text-muted-foreground mb-4">
          <span>
            {gitDiff.total_files} {gitDiff.total_files === 1 ? 'file' : 'files'}{' '}
            changed
          </span>
          <span>
            <span className="text-green-600">+{gitDiff.total_additions}</span>{' '}
            <span className="text-red-600">-{gitDiff.total_deletions}</span>
          </span>
        </div>
        {gitDiff.files.map((file) => (
          <GitFileDiff key={file.path} file={file} />
        ))}
      </div>
    </ScrollArea>
  );
}
