import { Injectable, signal, WritableSignal } from '@angular/core';

@Injectable({
  providedIn: 'root'
})
export class StateService {
  public static displayType: WritableSignal<'simple' | 'advanced'> = signal('simple')
}
