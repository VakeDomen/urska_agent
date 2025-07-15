import { Component, Input } from '@angular/core'
import { CommonModule } from '@angular/common'
import { trigger, style, transition, animate } from '@angular/animations'
import { Message } from '../../models/message.model'

@Component({
  selector: 'message-list',
  standalone: true,
  imports: [ CommonModule ],
  templateUrl: './message-list.component.html',
  styleUrls: ['./message-list.component.css'],
  animations: [
    trigger('fadeIn', [
      transition(':enter', [
        style({ opacity: 0 }),
        animate('200ms', style({ opacity: 1 }))
      ])
    ])
  ]
})
export class MessageListComponent {
  @Input() messages: Message[] = []
}
