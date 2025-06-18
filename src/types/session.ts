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
  session_id: string;
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

export interface SessionMetadata {
  session_id: string;
  created_at: string;
  last_message_at: string;
  agent_type: string;
  status: 'active' | 'completed' | 'error';
  total_messages: number;
  total_cost_usd: number;
}

export interface SessionMessages {
  messages: Array<ClaudeMessage>;
}

export interface Session {
  id: string;
  title: string;
  time: string;
  project: string;
  status: 'active' | 'completed' | 'error';
  changes?: { added: number; removed: number };
}
