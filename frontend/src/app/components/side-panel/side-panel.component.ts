import { Component, Input } from '@angular/core'
import { CommonModule } from '@angular/common'
import { Notification } from '../../models/notification.model'

@Component({
  selector: 'side-panel',
  standalone: true,
  imports: [ CommonModule ],
  templateUrl: './side-panel.component.html',
  styleUrls: ['./side-panel.component.css']
})
export class SidePanelComponent {
  @Input() notifications: Notification[] = []
  @Input() open = false

  toggle(n: Notification) {
    n.expanded = !n.expanded
  }
}
