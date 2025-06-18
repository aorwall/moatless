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
