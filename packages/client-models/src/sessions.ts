import {Store} from './store.ts';
import type {Invoke,SessionInfo} from '../../client/src/types.ts';
export class SessionsModel extends Store<SessionInfo[]> {
 private invoke:Invoke;
 constructor(invoke:Invoke){super([]);this.invoke=invoke;}
 refresh=()=>this.load(()=>this.invoke('session_list'));
 /** A reconnect baseline is authoritative, but cannot overwrite subsequent events. */
 reconnect=()=>this.refresh();
 update=(item:SessionInfo)=>this.set([item,...this.state.filter(x=>x.id!==item.id)]);
 async create(model=''){const generation=this.generation;const id=crypto.randomUUID();await this.invoke('session_create',{id});if(model)await this.invoke('session_set_model',{id,model});const item=await this.invoke('session_get',{id});if(generation!==this.generation)throw new Error('Client scope changed');this.update(item);return item;}
 private writes=Promise.resolve();
 setModel(id:string,model:string){const generation=this.generation;const next=this.writes.then(async()=>{await this.invoke('session_set_model',{id,model});if(generation!==this.generation)return;this.set(this.state.map(item=>item.id===id?{...item,model}:item));});this.writes=next.catch(()=>{});return next;}
 dispose=()=>this.clear([]);
}
