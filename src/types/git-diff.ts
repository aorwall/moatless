export interface DiffChunk {
  header: string;
  content: string;
  start_line_old: number;
  line_count_old: number;
  start_line_new: number;
  line_count_new: number;
}

export interface GitDiffFile {
  path: string;
  status: 'added' | 'modified' | 'deleted' | 'renamed';
  additions: number;
  deletions: number;
  old_path?: string;
  chunks: Array<DiffChunk>;
}

export interface GitDiffResponse {
  files: Array<GitDiffFile>;
  total_files: number;
  total_additions: number;
  total_deletions: number;
  commit_hash?: string;
  branch?: string;
}
