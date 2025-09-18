import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { StateService } from '../../state/state.service';

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

  constructor() {

  }

  toggleLogin(): void {
    this.isLoggedIn = !this.isLoggedIn;
    console.log(`User is now ${this.isLoggedIn ? 'Logged In' : 'Logged Out'}`);
  }

  toggleAdvanced(): void {
    if (StateService.displayType() == 'advanced') {
      StateService.displayType.set('simple')
    } else if (StateService.displayType() == 'simple') {
      StateService.displayType.set('advanced')
    }
    console.log(StateService.displayType())
    this.isAdvancedVisible = (StateService.displayType() == 'advanced')
  }

 
}