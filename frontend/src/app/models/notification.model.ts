export type NotificationContent =
  | { Done: [boolean, string] }
  | { PromptRequest: any } 
  | { PromptSuccessResult: any }
  | { PromptErrorResult: string }
  | { ToolCallRequest: { function: { name: string; arguments: any } } }
  | { ToolCallSuccessResult: string }
  | { ToolCallErrorResult: string }
  | { McpToolNotification: string }
  | { Token: { key: string, value: string } }
  | { Custom: { message: string } };
export interface BackendNotification {
  agent: string;
  content: NotificationContent;
  mcp_envelope: {
    progress_token: number;
    progress: number;
  } | null;
}

export interface Notification extends BackendNotification {
  id: number;
  expanded: boolean;
  rawExpanded: boolean;
  systemPromptVisible: boolean;
  taskVisible: boolean;
  arrivalTime: number;
  timeDelta?: number;
}