import { DiffModeEnum, DiffView } from '@git-diff-view/react';
import '@git-diff-view/react/styles/diff-view.css';
import { generateDiffFile } from '@git-diff-view/file';
import './git-diff-viewer.css';
import {
  ChevronRight,
  FileEdit,
  FileIcon,
  FileMinus,
  FilePlus,
  Loader2,
} from 'lucide-react';
import { useMemo } from 'react';

import { useTaskContext } from '@/features/tasks/contexts/task-context';
import { Badge } from '@/lib/components/ui/badge';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import { cn } from '@/lib/utils';
import type { GitDiffFile } from '@/types/git-diff';

interface GitDiffViewerProps {
  className?: string;
}

const getFileIcon = (status: GitDiffFile['status']) => {
  switch (status) {
    case 'added':
      return (
        <FilePlus className="h-4 w-4 text-green-600 dark:text-green-400" />
      );
    case 'deleted':
      return <FileMinus className="h-4 w-4 text-red-600 dark:text-red-400" />;
    case 'modified':
      return (
        <FileEdit className="h-4 w-4 text-yellow-600 dark:text-yellow-400" />
      );
    case 'renamed':
      return <FileIcon className="h-4 w-4 text-blue-600 dark:text-blue-400" />;
    default:
      return <FileIcon className="h-4 w-4" />;
  }
};

const getStatusColor = (status: GitDiffFile['status']) => {
  switch (status) {
    case 'added':
      return 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200';
    case 'deleted':
      return 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200';
    case 'modified':
      return 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200';
    case 'renamed':
      return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200';
    default:
      return 'bg-gray-100 text-gray-800 dark:bg-gray-900 dark:text-gray-200';
  }
};

export function GitDiffViewer({ className }: GitDiffViewerProps = {}) {
  const {
    gitDiff: contextGitDiff,
    selectedIteration,
    isLoadingGitDiff: isLoading,
    gitDiffError: error,
  } = useTaskContext();

  // Use iteration's git diff if available, otherwise use context's git diff
  const gitDiff = selectedIteration?.git_diff
    ? // Parse git diff string if it comes from selected iteration
      typeof selectedIteration.git_diff === 'string'
      ? null
      : // For now, we'll handle string diffs later
        selectedIteration.git_diff
    : contextGitDiff;
  const isDarkMode = useMemo(() => {
    return document.documentElement.classList.contains('dark');
  }, []);

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <Loader2 className="h-8 w-8 animate-spin mb-2" />
        <p>Loading diff...</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full p-8">
        <ErrorAlert
          error={error}
          title="Failed to load git diff"
          className="max-w-md"
        />
      </div>
    );
  }

  if (!gitDiff || gitDiff.files.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <FileIcon className="h-12 w-12 mb-2" />
        <p>No changes to display</p>
      </div>
    );
  }

  return (
    <div className={cn('h-full overflow-auto', className)}>
      {/* Summary */}
      <div className="sticky top-0 z-10 p-4 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
        <div className="flex items-center gap-4 text-sm">
          <span className="font-medium">
            {gitDiff.total_files} file{gitDiff.total_files !== 1 ? 's' : ''}{' '}
            changed
          </span>
          <span className="text-green-600 dark:text-green-400">
            +{gitDiff.total_additions}
          </span>
          <span className="text-red-600 dark:text-red-400">
            -{gitDiff.total_deletions}
          </span>
          {gitDiff.branch && (
            <span className="text-muted-foreground">on {gitDiff.branch}</span>
          )}
        </div>
      </div>

      {/* File Diffs */}
      <div className="divide-y">
        {gitDiff.files.map((file, index) => (
          <FileDiff
            key={`${file.path}-${index}`}
            file={file}
            isDarkMode={isDarkMode}
          />
        ))}
      </div>
    </div>
  );
}

interface FileDiffProps {
  file: GitDiffFile;
  isDarkMode: boolean;
}

function FileDiff({ file, isDarkMode }: FileDiffProps) {
  const defaultOpen = file.additions + file.deletions < 100;

  // Generate DiffFile using git-diff-view/file
  const diffFile = useMemo(() => {
    const oldContent = file.old_content || '';
    const newContent = file.new_content || '';
    const oldPath = file.old_path || file.path;
    const newPath = file.path;

    // Determine file extension for syntax highlighting
    const getFileExtension = (path: string) => {
      const parts = path.split('.');
      return parts.length > 1 ? parts[parts.length - 1] : '';
    };

    const extension = getFileExtension(newPath);

    // Generate the diff file
    const generatedFile = generateDiffFile(
      oldPath,
      oldContent,
      newPath,
      newContent,
      extension, // language for old file
      extension, // language for new file
    );

    // Initialize the diff file
    generatedFile.initTheme(isDarkMode ? 'dark' : 'light');
    generatedFile.init();
    generatedFile.buildSplitDiffLines();
    generatedFile.buildUnifiedDiffLines();

    return generatedFile;
  }, [file, isDarkMode]);

  return (
    <Collapsible defaultOpen={defaultOpen}>
      <CollapsibleTrigger className="w-full">
        <div className="flex items-center gap-2 p-4 hover:bg-muted/50 transition-colors">
          <ChevronRight className="h-4 w-4 transition-transform duration-200 data-[state=open]:rotate-90" />
          {getFileIcon(file.status)}
          <span className="font-mono text-sm flex-1 text-left truncate">
            {file.old_path && file.old_path !== file.path ? (
              <>
                <span className="line-through text-muted-foreground">
                  {file.old_path}
                </span>
                {' → '}
                <span>{file.path}</span>
              </>
            ) : (
              file.path
            )}
          </span>
          <Badge
            variant="secondary"
            className={cn('text-xs', getStatusColor(file.status))}
          >
            {file.status}
          </Badge>
          <div className="flex items-center gap-2 text-xs">
            <span className="text-green-600 dark:text-green-400">
              +{file.additions}
            </span>
            <span className="text-red-600 dark:text-red-400">
              -{file.deletions}
            </span>
          </div>
        </div>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="border-t">
          <div
            className={cn(
              'git-diff-view-wrapper',
              isDarkMode ? 'diff-view-theme-dark' : 'diff-view-theme-light',
            )}
          >
            <DiffView
              diffFile={diffFile}
              diffViewMode={DiffModeEnum.Unified}
              diffViewWrap={true}
              diffViewHighlight={true}
              diffViewFontSize={13}
            />
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
