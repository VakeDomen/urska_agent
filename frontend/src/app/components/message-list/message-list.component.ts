import { Component, Input, SecurityContext } from '@angular/core'
import { CommonModule } from '@angular/common'
import { trigger, style, transition, animate } from '@angular/animations'
import { Message } from '../../models/message.model'
import { MarkdownModule, MarkdownService, SECURITY_CONTEXT } from 'ngx-markdown'; 

@Component({
  selector: 'message-list',
  standalone: true,
  imports: [ CommonModule, MarkdownModule ],
  templateUrl: './message-list.component.html',
  styleUrls: ['./message-list.component.css'],
  providers: [
    MarkdownService,
    { provide: SECURITY_CONTEXT, useValue: SecurityContext.HTML },
  ],
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
