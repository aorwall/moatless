export interface GitDiffFile {
  path: string;
  status: 'added' | 'modified' | 'deleted' | 'renamed';
  additions: number;
  deletions: number;
  old_path?: string;
  old_content: string;
  new_content: string;
}

export interface GitDiffResponse {
  files: Array<GitDiffFile>;
  total_files: number;
  total_additions: number;
  total_deletions: number;
  commit_hash?: string;
  branch?: string;
}
