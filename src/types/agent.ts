export interface AgentOptions {
  [key: string]: unknown;
}

export interface AgentTriggerRequest {
  agent_type: string;
  message: string;
  environment_id: string;
}

export interface AgentTriggerResponse {
  session_id: string;
  status: string;
  stream_url?: string;
}
