import { FileCode2 } from 'lucide-react';

import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { GitDiffResponse } from '@/types/git-diff';

import { GitFileDiff } from './git-file-diff';

interface DiffViewerProps {
  gitDiff?: GitDiffResponse;
}

export function DiffViewer({ gitDiff }: DiffViewerProps) {
  if (!gitDiff || gitDiff.files.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <div className="text-center space-y-3">
          <FileCode2 className="h-8 w-8 mx-auto text-muted-foreground/50" />
          <p className="text-muted-foreground">No changes to display</p>
        </div>
      </div>
    );
  }

  return (
    <ScrollArea className="h-full w-full">
      <div className="p-3 sm:p-4 lg:p-6 space-y-4 w-full">
        <div className="flex items-center justify-between text-sm text-muted-foreground mb-4 w-full flex-wrap gap-2">
          <span>
            {gitDiff.total_files} {gitDiff.total_files === 1 ? 'file' : 'files'}{' '}
            changed
          </span>
          <span className="flex items-center gap-2">
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
