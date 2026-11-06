import {Store} from './store.ts';
import type {Invoke,Provider,CodexModel} from '../../client/src/types.ts';
export class ProvidersModel extends Store<Provider[]> {
 readonly codex=new Store<CodexModel[]>([]);
 private invoke:Invoke;
 constructor(invoke:Invoke){super([]);this.invoke=invoke;}
 refresh=()=>this.load(()=>this.invoke('model_providers'));
 refreshCodex=()=>this.codex.load(()=>this.invoke('codex_models'));
 dispose=()=>{this.clear([]);this.codex.clear([]);};
}
