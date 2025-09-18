import { bootstrapApplication } from '@angular/platform-browser'
import { provideAnimations } from '@angular/platform-browser/animations'
import { ChatComponent } from './app/components/chat/chat.component'
import { NavbarComponent } from './app/components/navbar/navbar.component'
import { provideZonelessChangeDetection } from '@angular/core'

bootstrapApplication(ChatComponent, {
  providers: [ provideAnimations(), provideZonelessChangeDetection() ],
}).catch(err => console.error(err))

bootstrapApplication(NavbarComponent, {
  providers: [ provideAnimations(), provideZonelessChangeDetection() ],
}).catch(err => console.error(err))