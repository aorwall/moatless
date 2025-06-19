export interface TaskChanges {
  added: number
  removed: number
}

export interface Task {
  id: number
  title: string
  time: string
  project: string
  status: "active" | "completed" | "archived"
  changes?: TaskChanges
}

export interface FileChange {
  name: string
  status: "New" | "Modified" | "Deleted"
  changes: TaskChanges
  diff: string
}

export interface TaskDetail {
  id: string
  title: string
  date: string
  project: string
  branch: string
  changes: TaskChanges
  description: string
  duration: string
  notes: string[]
  summary: string[]
  testing: {
    status: "passed" | "failed" | "pending"
    details: string
  }
  files: FileChange[]
}

export interface GitFileChange {
  path: string
  status: "added" | "modified" | "deleted" | "renamed"
  additions: number
  deletions: number
  old_path?: string
  chunks: Array<{
    header: string
    content: string
    start_line_old: number
    line_count_old: number
    start_line_new: number
    line_count_new: number
  }>
}

export interface GitDiff {
  files: GitFileChange[]
  total_files: number
  total_additions: number
  total_deletions: number
  commit_hash?: string
  branch?: string
}

export interface TaskIteration {
  iteration_id?: string
  iteration_number: number
  start_time: string
  end_time?: string
  status: "active" | "completed" | "failed" | "cancelled"
  message_count: number
  cost_usd: number
  initial_message: string
  error_message?: string
  result?: string
  duration_ms: number
  git_diff?: GitDiff
}

export interface TaskMetadata {
  task_id: string
  title?: string
  created_at: string
  last_message_at?: string
  agent_type: string
  environment_id: string
  environment_name: string
  status: string
  num_turns: number
  total_cost_usd: number
  is_running: boolean
  iterations: TaskIteration[]
}

export interface ClaudeMessage {
  id: string
  content: string
  role: "user" | "assistant"
  timestamp: string
}
