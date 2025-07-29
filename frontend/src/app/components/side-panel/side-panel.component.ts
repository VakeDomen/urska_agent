import { Component, Input, SecurityContext } from '@angular/core';
import { CommonModule, JsonPipe } from '@angular/common';
import { Notification, NotificationContent } from '../../models/notification.model';
import { MarkdownModule, MarkdownService, SECURITY_CONTEXT } from 'ngx-markdown';

@Component({
  selector: 'side-panel',
  standalone: true,
  imports: [CommonModule, JsonPipe, MarkdownModule],
  providers: [
    MarkdownService,
    { provide: SECURITY_CONTEXT, useValue: SecurityContext.HTML },
  ],
  templateUrl: './side-panel.component.html',
  styleUrls: ['./side-panel.component.css']
})
export class SidePanelComponent {
  @Input() notifications: Notification[] = [];

  toggle(n: Notification) {
    n.expanded = !n.expanded;
  }
  
  // Toggles the visibility of the system prompt inside a PromptRequest
  toggleSystemPrompt(n: Notification): void {
    n.systemPromptVisible = !n.systemPromptVisible;
  }

   // Toggles the visibility of the system prompt inside a PromptRequest
  toggleTask(n: Notification): void {
    n.taskVisible = !n.taskVisible;
  }

  // Helper to provide a short summary for collapsed notifications
  getNotificationSummary(content: NotificationContent): string {
    if ('ToolCallRequest' in content) {
      return `Tool Call: ${content.ToolCallRequest.function.name}`;
    }

    if ('Done' in content) {
      return `Done: ${content.Done[0]}`
    }

    if ('PromptRequest' in content) {
      return `Prompt Request`
    }

    if ('PromptSuccessResult' in content) {
      return "Prompt Response"
    }

    if ('PromptErrorResult' in content) {
      return `Prompt Error`
    }

    if ('ToolCallSuccessResult' in content) {
      return "Tool Response"
    }

    if ('ToolCallErrorResult' in content) {
      return "Tool Error"
    }

    if ('McpToolNotification' in content) {
      return "MCP Nortification"
    }

    return 'General Notification';
  }

  // Gets the key of the notification content object for use in ngSwitch
  getNotificationType(content: NotificationContent): string {
    return Object.keys(content)[0];
  }

  // Toggles the raw JSON view
  toggleRaw(n: Notification): void {
    n.rawExpanded = !n.rawExpanded;
  }

  trackById(index: number, item: Notification): number {
    return item.id;
  }

  // Extracts the user's task from a PromptRequest messages array
  getUserTask(messages: any[]): string {
    if (!messages || messages.length === 0) return 'No task found.';
    const userMessage = messages.slice().reverse().find(m => m.role === 'user');
    return userMessage?.content ?? 'No user message found.';
  }
}