import { Component, OnInit, ChangeDetectorRef, ChangeDetectionStrategy, effect, computed } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MessageListComponent } from '../message-list/message-list.component';
import { ChatInputComponent } from '../chat-input/chat-input.component';
import { SidePanelComponent } from '../side-panel/side-panel.component';
import { CountedError, CountedToken, Message } from '../../models/message.model';
import { Notification, BackendNotification } from '../../models/notification.model';
import { StateService } from '../../state/state.service';
import { UserProfile } from '../../models/profile.model';
import { LoginModalComponent } from "../login/login.component";

@Component({
  selector: 'app-chat',
  standalone: true,
  imports: [
    CommonModule,
    MessageListComponent,
    ChatInputComponent,
    SidePanelComponent,
    LoginModalComponent
],
  templateUrl: './chat.component.html',
  styleUrls: ['./chat.component.css'],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ChatComponent implements OnInit {
  public messages: Message[] = [];
  public notifications: Notification[] = [];
  public resultNotifications: Notification[] = [];
  public stateMessage: String | undefined;
  public errorMessage: CountedError | undefined;
  public queuePosition: number = 0;
  public lastToken: CountedToken | undefined;
  public leftSideOpen = true;
  public rightSideOpen = true;
  public displayAdvanced = false;
  public isProcessing = false;
  public isLoggedIn = false;
  public socket: WebSocket = new WebSocket('ws://localhost:8080/ws');
  public socketStatus: 'connecting' | 'open' | 'closed' = 'connecting';
  public tokenCount: number = 0;

  constructor(
    private cdr: ChangeDetectorRef,
  ) { 
    effect(() => {
      this.displayAdvanced = (StateService.displayType() == 'advanced')
      const wasLoggedIn = this.isLoggedIn;
      this.isLoggedIn = !!StateService.userProfile(); 

      if (wasLoggedIn && !this.isLoggedIn) {
        this.socket.send(JSON.stringify({ 
          message_type: "Logout",
          content: "" 
        }));
      }

      this.cdr.detectChanges();
    });
  }

  ngOnInit() {
    this.socket.onopen = () => {
      this.socketStatus = 'open';
      this.cdr.detectChanges();
    };

    this.socket.onclose = () => {
      this.socketStatus = 'closed';
      this.cdr.detectChanges();
    };

    this.socket.onerror = () => {
      this.socketStatus = 'closed';
      this.cdr.detectChanges();
    };

    this.socket.onmessage = (ev: MessageEvent) => {
      const msg = JSON.parse(ev.data);

      if (msg.type === 'Error') {
        this.handleErrorMessage(msg);
      }

      if (msg.type === 'LoginProfile') {
        this.handleLoginProfileMessage(msg)
      }

      if (msg.type === "QueuePosition") {
        this.handleQueuePositionMessage(msg);
      }


      if (msg.type === 'Notification') {
        this.handleNotificationMessage(msg);
      }

      this.cdr.detectChanges();
    };
  }
  handleNotificationMessage(msg: any) {
    this.queuePosition = 0;

    const backendNotification = JSON.parse(msg.data) as BackendNotification;
    if (
      backendNotification.agent === 'Urška' &&
      'Token' in backendNotification.content
    ) {
      this.lastToken = {
        value: backendNotification.content.Token.value,
        seq: this.tokenCount++
      } as CountedToken;
      
      this.cdr.detectChanges();
      return
    }

    if ('Token' in backendNotification.content) {
      return;
    }


    if ('Custom' in backendNotification.content) {
      this.stateMessage = backendNotification.content.Custom.message;
    }

    if (
      backendNotification.agent === 'Urška' &&
      'Done' in backendNotification.content &&
      Array.isArray(backendNotification.content.Done)
    ) {
      // It's the final answer. Add it to chat and close the panel.
      this.isProcessing = false;
      console.log("Done processing")
      // this.rightSideOpen = false;
      // this.leftSideOpen = false;
    } else {
      const arrivalTime = Date.now();
      const lastNotification = this.notifications[this.notifications.length - 1];
      const timeDelta = lastNotification
        ? (arrivalTime - lastNotification.arrivalTime) / 1000
        : undefined;

      const notification = {
        ...backendNotification,
        id: this.notifications.length,
        expanded: false,
        rawExpanded: false,
        systemPromptVisible: false,
        taskVisible: false,
        arrivalTime,
        timeDelta,
      }

      if ('ToolCallSuccessResult' in backendNotification.content) {
        this.leftSideOpen = true;
        this.resultNotifications.push(notification)
      } else {
        this.notifications.push(notification)
      }

    }
  }

  sendPrompt(prompt: string) {
    if (!prompt.trim() || this.isProcessing) return;

    this.messages.push({
      role: 'user',
      content: prompt,
      timestamp: new Date(),
      error: undefined,
      state: undefined,
    });

    this.messages.push({
      role: 'assistant',
      content: "",
      timestamp: new Date(),
      error: undefined,
      state: undefined
    })

    this.notifications = [];
    this.isProcessing = true;
    this.rightSideOpen = true;

    this.socket.send(JSON.stringify({ 
      message_type: "Prompt",
      content: prompt 
    }));
  }

  handleRightPanelEnter() {
    this.rightSideOpen = true;
  }

  handleRightPanelLeave() {
    if (!this.isProcessing) {
      // this.rightSideOpen = false;
    }
  }

  handleLeftPanelEnter() {
    this.leftSideOpen = true;
  }

  handleLeftPanelLeave() {
    if (!this.isProcessing) {
      // this.leftSideOpen = false;
    }
  }


  handleQueuePositionMessage(msg: any) {
    this.queuePosition = +msg.data;
  }


  handleErrorMessage(msg: any) {
    this.errorMessage = {
      seq: this.tokenCount++,
      value: msg.data
    } as CountedError;
    this.isProcessing = false;
  }

  handleLoginProfileMessage(data: UserProfile) {
    StateService.userProfile.set(data);
  }
}
