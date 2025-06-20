export enum MessageType {
  USER = 'user',
  ASSISTANT = 'assistant',
  RESULT = 'result',
}

export enum Role {
  USER = 'user',
  ASSISTANT = 'assistant',
  SYSTEM = 'system',
}

export enum ContentType {
  TEXT = 'text',
  TOOL_RESULT = 'tool_result',
  TOOL_USE = 'tool_use',
}

export enum ResultSubtype {
  SUCCESS = 'success',
  ERROR = 'error',
}

export interface TextContent {
  type: ContentType.TEXT;
  text: string;
}

export interface ToolResultContent {
  type: ContentType.TOOL_RESULT;
  content: string;
  is_error: boolean;
  tool_use_id: string;
}

export interface ToolUseInput {
  [key: string]: unknown;
}

export interface ToolUseContent {
  type: ContentType.TOOL_USE;
  id: string;
  name: string;
  input: ToolUseInput;
}

export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
  service_tier?: string;
}

export interface MessageContent {
  id?: string;
  type: string;
  role: Role;
  model?: string;
  content: Array<TextContent | ToolResultContent | ToolUseContent>;
  stop_reason?: string;
  stop_sequence?: string;
  usage?: Usage;
}

export interface BaseMessage {
  type: MessageType;
  parent_tool_use_id?: string;
  task_id: string;
  timestamp: string;
}

export interface UserMessage extends BaseMessage {
  type: MessageType.USER;
  message: MessageContent;
}

export interface AssistantMessage extends BaseMessage {
  type: MessageType.ASSISTANT;
  message: MessageContent;
}

export interface ResultMessage extends BaseMessage {
  type: MessageType.RESULT;
  subtype: ResultSubtype;
  is_error: boolean;
  duration_ms: number;
  duration_api_ms: number;
  num_turns: number;
  result: string;
  total_cost_usd: number;
  usage: Usage;
}

export type ClaudeMessage = UserMessage | AssistantMessage | ResultMessage;

export interface TaskIteration {
  iteration_id: string;
  start_time: string;
  end_time: string | null;
  status: 'active' | 'completed' | 'error';
  message_count: number;
  cost_usd: number;
  initial_message: string;
  error_message: string | null;
  result: string | null;
  duration_ms: number;
  git_diff: string | null;
}

export interface TaskMetadata {
  task_id: string;
  title: string;
  created_at: string;
  last_message_at: string;
  agent_type: string;
  environment_id: string;
  is_running: boolean;
  status: 'active' | 'completed' | 'error';
  num_turns: number;
  total_cost_usd: number;
  iterations: Array<TaskIteration>;
}

export interface TaskMessages {
  messages: Array<ClaudeMessage>;
}

export interface Task {
  id: string;
  title: string;
  time: string;
  project: string;
  status: 'active' | 'completed' | 'error';
  changes?: { added: number; removed: number };
}

export interface FileDiff {
  name: string;
  status: 'Modified' | 'New' | 'Deleted';
  changes: { added: number; removed: number };
  diff: string;
}

export interface TaskDetail {
  id: number;
  title: string;
  time: string;
  date: string;
  project: string;
  status: 'active' | 'completed' | 'error';
  branch: string;
  changes: { added: number; removed: number };
  description: string;
  duration: string;
  notes: Array<string>;
  summary: Array<string>;
  testing: {
    status: 'passed' | 'failed' | 'pending';
    details: string;
  };
  files: Array<FileDiff>;
}
