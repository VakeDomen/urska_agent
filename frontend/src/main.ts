import { bootstrapApplication } from '@angular/platform-browser'
import { provideAnimations } from '@angular/platform-browser/animations'
import { ChatComponent } from './app/components/chat/chat.component'
import { provideZonelessChangeDetection } from '@angular/core'

bootstrapApplication(ChatComponent, {
  providers: [ provideAnimations(), provideZonelessChangeDetection() ],
}).catch(err => console.error(err))