import { Injectable, signal, WritableSignal } from '@angular/core';
import { UserProfile } from '../models/profile.model';

@Injectable({
  providedIn: 'root'
})
export class StateService {
  public static displayType: WritableSignal<'simple' | 'advanced'> = signal('simple')
  public static userProfile: WritableSignal<UserProfile | null> = signal(null);
}
