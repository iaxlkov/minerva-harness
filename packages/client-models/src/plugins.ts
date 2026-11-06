import {Store} from './store.ts';
import type {Invoke,PluginInfo} from '../../client/src/types.ts';
export class PluginsModel extends Store<PluginInfo[]> {
 private invoke:Invoke;
 constructor(invoke:Invoke){super([]);this.invoke=invoke;}
 refresh=()=>this.load(()=>this.invoke('list_plugins'));
 async toggle(id:string,enabled:boolean){const generation=this.generation;const items=await this.invoke('set_plugin_enabled',{id,enabled});if(generation===this.generation)this.set(items);}
 dispose=()=>this.clear([]);
}
