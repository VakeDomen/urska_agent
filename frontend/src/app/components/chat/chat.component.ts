import { Component, OnInit, ChangeDetectorRef } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MessageListComponent } from '../message-list/message-list.component';
import { ChatInputComponent } from '../chat-input/chat-input.component';
import { SidePanelComponent } from '../side-panel/side-panel.component';
import { Message } from '../../models/message.model';
import { Notification, BackendNotification } from '../../models/notification.model';

@Component({
  selector: 'app-chat',
  standalone: true,
  imports: [
    CommonModule,
    MessageListComponent,
    ChatInputComponent,
    SidePanelComponent,
  ],
  templateUrl: './chat.component.html',
  styleUrls: ['./chat.component.css']
})
export class ChatComponent implements OnInit {
  messages: Message[] = [];
  notifications: Notification[] = [];
  sideOpen = false;
  isProcessing = false; // New state to lock the panel open during a request
  private socket: WebSocket = new WebSocket('ws://localhost:8080/ws');

  constructor(private cdr: ChangeDetectorRef) {}

  ngOnInit() {
    this.socket.onmessage = (ev: MessageEvent) => {
        const msg = JSON.parse(ev.data);

        if (msg.type === 'Notification') {
          const backendNotification = JSON.parse(msg.data) as BackendNotification;

          // Check if this is the final 'Done' notification
          if (
            backendNotification.agent === 'Urška' && // Changed from '===' to 'startsWith' for more flexibility
            'Done' in backendNotification.content &&
            Array.isArray(backendNotification.content.Done)
          ) {
            // It's the final answer. Add it to chat and close the panel.
            this.messages.push({
              role: 'assistant',
              content: backendNotification.content.Done[1],
              timestamp: new Date(),
            });
            this.isProcessing = false;
            this.sideOpen = false;
          } else {
            // It's a regular progress notification
            this.notifications.push({
              ...backendNotification,
              id: this.notifications.length,
              expanded: false, 
              rawExpanded: false,
              systemPromptVisible: false,
              taskVisible: false,
            });
          }
        }
        
        setTimeout(() => {
          this.cdr.detectChanges();
        });
    };
  }

  sendPrompt(prompt: string) {
    if (!prompt.trim() || this.isProcessing) return;

    this.messages.push({
      role: 'user',
      content: prompt,
      timestamp: new Date(),
    });
    
    // Clear old notifications and open the panel for the new request
    this.notifications = [];
    this.isProcessing = true;
    this.sideOpen = true;

    this.socket.send(JSON.stringify({ question: prompt }));
  }

  // --- Hover Handlers for Manual Control ---
  handlePanelEnter() {
    this.sideOpen = true;
  }

  handlePanelLeave() {
    // Only close the panel on mouse leave if we are not actively processing a request.
    if (!this.isProcessing) {
      this.sideOpen = false;
    }
  }
}