import {Store} from './store.ts';
import type {Invoke,Provider} from '../../client/src/types.ts';
export class ProvidersModel extends Store<Provider[]> {
 private invoke:Invoke;
 constructor(invoke:Invoke){super([]);this.invoke=invoke;}
 refresh=()=>this.load(()=>this.invoke('model_providers'));
 dispose=()=>{this.clear([]);};
}
