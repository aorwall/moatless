import {
  ChevronRight,
  Columns2,
  FileEdit,
  FileIcon,
  FileMinus,
  FilePlus,
  Loader2,
  Square,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import ReactDiffViewer from 'react-diff-viewer-continued';

import { useTaskContext } from '@/features/tasks/contexts/task-context';
import { Badge } from '@/lib/components/ui/badge';
import { Button } from '@/lib/components/ui/button';
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip';
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
  const [splitView, setSplitView] = useState(false);
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
      {/* Summary and Controls */}
      <div className="sticky top-0 z-10 p-4 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
        <div className="flex items-center justify-between">
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
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setSplitView(!splitView)}
                  className="gap-2"
                >
                  {splitView ? (
                    <Square className="h-4 w-4" />
                  ) : (
                    <Columns2 className="h-4 w-4" />
                  )}
                  {splitView ? 'Unified' : 'Split'}
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                <p>Toggle between split and unified view</p>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </div>
      </div>

      {/* File Diffs */}
      <div className="divide-y">
        {gitDiff.files.map((file, index) => (
          <FileDiff
            key={`${file.path}-${index}`}
            file={file}
            isDarkMode={isDarkMode}
            splitView={splitView}
          />
        ))}
      </div>
    </div>
  );
}

interface FileDiffProps {
  file: GitDiffFile;
  isDarkMode: boolean;
  splitView: boolean;
}

function FileDiff({ file, isDarkMode, splitView }: FileDiffProps) {
  const isNewFile = file.status === 'added';
  const isDeletedFile = file.status === 'deleted';
  const defaultOpen = file.additions + file.deletions < 100;

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
          <ReactDiffViewer
            key={`${file.path}-${file.old_content?.length || 0}-${file.new_content?.length || 0}`}
            oldValue={isNewFile ? '' : file.old_content}
            newValue={isDeletedFile ? '' : file.new_content}
            splitView={splitView}
            useDarkTheme={isDarkMode}
            showDiffOnly={false}
            styles={{
              variables: {
                light: {
                  diffViewerBackground: '#ffffff',
                  addedBackground: '#e6ffec',
                  addedColor: '#24292e',
                  removedBackground: '#ffebe9',
                  removedColor: '#24292e',
                  wordAddedBackground: '#acf2bd',
                  wordRemovedBackground: '#fdb8c0',
                  addedGutterBackground: '#cdffd8',
                  removedGutterBackground: '#ffdce0',
                  gutterBackground: '#f6f8fa',
                  gutterBackgroundDark: '#f0f1f3',
                  highlightBackground: '#fffbdd',
                  highlightGutterBackground: '#fff5b1',
                  codeFoldGutterBackground: '#dbedff',
                  codeFoldBackground: '#f1f8ff',
                  emptyLineBackground: '#fafbfc',
                  gutterColor: '#666666',
                  addedGutterColor: '#212529',
                  removedGutterColor: '#212529',
                  codeFoldContentColor: '#212529',
                  diffViewerTitleBackground: '#fafbfc',
                  diffViewerTitleColor: '#212529',
                  diffViewerTitleBorderColor: '#eee',
                },
                dark: {
                  diffViewerBackground: '#0d1117',
                  addedBackground: '#033a16',
                  addedColor: '#e6edf3',
                  removedBackground: '#67060c',
                  removedColor: '#e6edf3',
                  wordAddedBackground: '#1c6b48',
                  wordRemovedBackground: '#a0111f',
                  addedGutterBackground: '#034419',
                  removedGutterBackground: '#5d0f12',
                  gutterBackground: '#161b22',
                  gutterBackgroundDark: '#0d1117',
                  highlightBackground: '#a08c14',
                  highlightGutterBackground: '#8a7613',
                  codeFoldGutterBackground: '#21262d',
                  codeFoldBackground: '#161b22',
                  emptyLineBackground: '#161b22',
                  gutterColor: '#8b949e',
                  addedGutterColor: '#e6edf3',
                  removedGutterColor: '#e6edf3',
                  codeFoldContentColor: '#8b949e',
                  diffViewerTitleBackground: '#161b22',
                  diffViewerTitleColor: '#e6edf3',
                  diffViewerTitleBorderColor: '#30363d',
                },
              },
              diffContainer: {
                fontFamily:
                  'ui-monospace, SFMono-Regular, SF Mono, Consolas, Liberation Mono, Menlo, monospace',
                fontSize: '13px',
              },
              line: {
                padding: '0 10px',
                '&:hover': {
                  background: isDarkMode ? '#1c2128' : '#f6f8fa',
                },
              },
              gutter: {
                minWidth: '50px',
                padding: '0 10px',
              },
              contentText: {
                // Override to ensure syntax highlighting is preserved
                '& pre': {
                  background: 'transparent !important',
                  margin: '0 !important',
                  padding: '0 !important',
                },
              },
            }}
            leftTitle={file.old_path || file.path}
            rightTitle={file.path}
            compareMethod="diffWords"
          />
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
