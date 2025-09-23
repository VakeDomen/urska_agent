import { Component, Input, OnChanges, SimpleChanges } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { CountedError } from '../../models/message.model';

@Component({
  selector: 'app-login-modal',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './login.component.html',
  styleUrls: ['./login.component.css']
})
export class LoginModalComponent implements OnChanges {
  @Input() socket: WebSocket | undefined;
  @Input() countedError: CountedError | undefined;
  
  username = '';
  password = '';
  error = '';
  

  ngOnChanges(changes: SimpleChanges): void {
    if (changes['countedError'] && changes['countedError'].currentValue) {
      this.error = changes['countedError'].currentValue.value;
    }
  }


  login(): void {
    if (!this.username.trim() || !this.password.trim()) {
      this.error = 'Username and password are required.';
      return;
    }
    
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
        this.error = 'Not connected to the server. Please wait.';
        return;
    }
    
    this.error = '';

    const loginPayload = {
      message_type: 'StudentLogin',
      content: JSON.stringify({
        username: this.username,
        password: this.password,
      }),
    };

    this.socket.send(JSON.stringify(loginPayload));
  }
}