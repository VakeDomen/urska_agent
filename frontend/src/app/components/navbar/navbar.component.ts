import { ChangeDetectorRef, Component, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { StateService } from '../../state/state.service';
import { LoginModalComponent } from "../login/login.component";

@Component({
  selector: 'app-navbar',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './navbar.component.html',
  styleUrl: './navbar.component.css'
})
export class NavbarComponent {
  pageTitle: string = 'Univerzitetne Rešitve: Študentski Komunikacijski Agent';
  isLoggedIn: boolean = false;
  isAdvancedVisible: boolean = false;

  constructor(private cdr: ChangeDetectorRef) {
    effect(() => {
      this.isLoggedIn = !!StateService.userProfile(); 
      this.cdr.detectChanges();
    });
  }

  toggleLogin(): void {
    if (this.isLoggedIn) {
      StateService.userProfile.set(null);
      console.log(`User is now Logged Out`);
    }
  }

  toggleAdvanced(): void {
    if (StateService.displayType() == 'advanced') {
      StateService.displayType.set('simple')
    } else if (StateService.displayType() == 'simple') {
      StateService.displayType.set('advanced')
    }
    console.log(StateService.displayType())
  }

 
}