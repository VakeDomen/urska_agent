import { Component, Output, EventEmitter, Input } from "@angular/core";
import { CommonModule } from "@angular/common";
import { FormsModule } from "@angular/forms";

@Component({
  selector: "chat-input",
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: "./chat-input.component.html",
  styleUrls: ["./chat-input.component.css"],
})
export class ChatInputComponent {
  text = "";
  @Output() send = new EventEmitter<string>();
  @Input() connectionStatus: "connecting" | "open" | "closed" = "closed";

  onEnter(event: any) {
    if (!event.shiftKey) {
      event.preventDefault();
      this.emit();
    }
  }

  emit() {
    if (!this.text.trim()) return;
    this.send.emit(this.text);
    this.text = "";
  }
}
