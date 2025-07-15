import { Component, OnInit } from '@angular/core'
import { CommonModule } from '@angular/common'
import { MessageListComponent } from '../message-list/message-list.component'
import { ChatInputComponent } from '../chat-input/chat-input.component'
import { SidePanelComponent } from '../side-panel/side-panel.component'
import { Message } from '../../models/message.model'
import { Notification } from '../../models/notification.model'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { SSEClientTransport } from '@modelcontextprotocol/sdk/client/sse.js'

@Component({
  selector: 'app-root',
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
  messages: Message[] = []
  notifications: Notification[] = []
  sideOpen = false
  private socket: WebSocket = new WebSocket('ws://localhost:8080/ws');


  ngOnInit() {
    this.socket.onmessage = (ev: any) => {
      const msg = JSON.parse(ev.data) as
        | { type: 'Chunk'; data: string }
        | { type: "Progress" ; data: string}
        | { type: 'End' }
      
      if (msg.type === 'Progress') {
        this.notifications.push({
          id: this.notifications.length,
          content: msg.data,
          expanded: false
        })
      } else if (msg.type === "Chunk") {
        const assistantMsg = { role: 'assistant', content: '', timestamp: new Date() } as Message
        assistantMsg.content += msg.data;
        this.messages.push(assistantMsg)
      }
    };
  }

  

  sendPrompt(prompt: string) {
    this.socket.send(JSON.stringify({ question: prompt }));
  }
}
