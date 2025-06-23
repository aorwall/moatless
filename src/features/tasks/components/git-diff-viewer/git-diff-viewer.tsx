import { useParams } from '@tanstack/react-router';
import { ChevronDown, ChevronRight, FileIcon, Loader2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { Diff, Hunk, parseDiff } from 'react-diff-view';
import 'react-diff-view/style/index.css';

import { useTaskGitDiff } from '@/features/tasks/hooks/use-tasks';
import { useSelectedIteration } from '@/features/tasks/stores/task-ui-store';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import { cn } from '@/lib/utils';
import './github-diff.css';

interface GitDiffViewerProps {
  className?: string;
}

// Using any for hunk types as react-diff-view doesn't provide complete type exports
interface ParsedFile {
  oldRevision: string;
  newRevision: string;
  type: 'add' | 'delete' | 'modify' | 'rename';
  hunks: any[];
  oldPath?: string;
  newPath?: string;
  [key: string]: any;
}

export function GitDiffViewer({ className }: GitDiffViewerProps) {
  const params = useParams({ from: '/tasks/$taskId' });
  const {
    data: contextGitDiff,
    isLoading,
    error,
  } = useTaskGitDiff(params.taskId);
  const selectedIteration = useSelectedIteration();

  // Use iteration's git diff if available, otherwise use context's git diff
  const gitDiff =
    selectedIteration?.git_diff &&
    typeof selectedIteration.git_diff === 'object'
      ? selectedIteration.git_diff
      : contextGitDiff;

  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());

  // Parse the git diff using gitdiff-parser - must be called before any returns for hooks rules
  const parsedFiles = useMemo(() => {
    if (!gitDiff?.diff) {
      return [];
    }
    try {
      const files = parseDiff(gitDiff.diff);
      // Auto-expand all files by default
      const allFileKeys = new Set(
        files.map(
          (f) => `${f.oldPath || f.oldRevision}-${f.newPath || f.newRevision}`,
        ),
      );
      setExpandedFiles(allFileKeys);
      return files;
    } catch (error) {
      console.error('Failed to parse git diff:', error);
      return [];
    }
  }, [gitDiff?.diff]);

  const toggleFileExpansion = (fileKey: string) => {
    setExpandedFiles((prev) => {
      const next = new Set(prev);
      if (next.has(fileKey)) {
        next.delete(fileKey);
      } else {
        next.add(fileKey);
      }
      return next;
    });
  };

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <Loader2
          className="h-8 w-8 animate-spin mb-2"
          data-testid="loading-spinner"
        />
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

  if (!gitDiff?.diff) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <FileIcon className="h-12 w-12 mb-2" />
        <p>No changes to display</p>
      </div>
    );
  }

  if (parsedFiles.length === 0 && gitDiff.total_files === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <FileIcon className="h-12 w-12 mb-2" />
        <p>No changes to display</p>
      </div>
    );
  }

  if (parsedFiles.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <FileIcon className="h-12 w-12 mb-2" />
        <p>Unable to parse diff</p>
      </div>
    );
  }

  const renderFile = (file: ParsedFile) => {
    const { oldRevision, newRevision, type, hunks, oldPath, newPath } = file;
    const filePath = newPath || oldPath || newRevision || oldRevision;
    const fileKey = `${oldPath || oldRevision}-${newPath || newRevision}`;
    const isExpanded = expandedFiles.has(fileKey);

    // Calculate additions and deletions
    let additions = 0;
    let deletions = 0;

    hunks.forEach((hunk) => {
      hunk.changes.forEach((change: any) => {
        if (change.type === 'insert') {
          additions++;
        }
        if (change.type === 'delete') {
          deletions++;
        }
      });
    });

    const stats = { additions, deletions };

    return (
      <div
        key={fileKey}
        className="github-diff-file border border-border rounded-lg mb-4 overflow-hidden"
      >
        {/* File Header */}
        <button
          type="button"
          className="github-diff-header bg-muted/50 px-4 py-2 flex items-center justify-between cursor-pointer hover:bg-muted/70 transition-colors w-full text-left"
          onClick={() => toggleFileExpansion(fileKey)}
        >
          <div className="flex items-center gap-2">
            <span className="p-0 hover:bg-muted rounded">
              {isExpanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
            </span>
            <FileIcon className="h-4 w-4 text-muted-foreground" />
            <span className="font-mono text-sm">{filePath}</span>
          </div>
          <div className="flex items-center gap-3 text-sm">
            <span className="text-green-600 dark:text-green-400">
              +{stats.additions}
            </span>
            <span className="text-red-600 dark:text-red-400">
              −{stats.deletions}
            </span>
          </div>
        </button>

        {/* Diff Content */}
        {isExpanded && (
          <div className="github-diff-content">
            <Diff
              viewType="unified"
              diffType={type}
              hunks={hunks}
              className="diff-view"
            >
              {(hunks) =>
                hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)
              }
            </Diff>
          </div>
        )}
      </div>
    );
  };

  return (
    <div className={cn('h-full overflow-auto p-4', className)}>
      {/* Summary Stats */}
      {gitDiff && (
        <div className="mb-4 text-sm text-muted-foreground">
          Showing {parsedFiles.length} changed files with{' '}
          <span className="text-green-600 dark:text-green-400 font-medium">
            {gitDiff.total_additions} additions
          </span>{' '}
          and{' '}
          <span className="text-red-600 dark:text-red-400 font-medium">
            {gitDiff.total_deletions} deletions
          </span>
        </div>
      )}

      {/* File Diffs */}
      <div>
        {parsedFiles.map((file) => renderFile(file as ParsedFile))}
      </div>
    </div>
  );
}
