export interface Task {
  id: number;
  title: string;
  time: string;
  project: string;
  status: string;
  changes?: { added: number; removed: number };
}

export interface TaskDetail {
  id: number;
  title: string;
  time: string;
  project: string;
  status: string;
  changes: { added: number; removed: number };
  date: string;
  branch: string;
  description: string;
  duration: string;
  notes: Array<string>;
  summary: Array<string>;
  testing: {
    status: string;
    details: string;
  };
  files: Array<FileDiff>;
}

export interface FileDiff {
  name: string;
  status: 'New' | 'Modified' | 'Deleted';
  changes: { added: number; removed: number };
  diff: string;
}
