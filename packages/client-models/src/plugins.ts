import {Store} from './store.ts';
import type {Invoke,PluginInfo} from '../../client/src/types.ts';
export class PluginsModel extends Store<PluginInfo[]> {
 private invoke:Invoke;
 readonly errors=new Store('');
 private refreshing:Promise<void>|null=null;
 private pendingToggles=0;
 private stopPolling:(()=>void)|null=null;
 constructor(invoke:Invoke){super([]);this.invoke=invoke;}
 refresh=():Promise<void>=>{
  if(this.pendingToggles)return Promise.resolve();
  if(this.refreshing)return this.refreshing;
  const revision=++this.revision;
  this.refreshing=this.invoke('list_plugins').then(items=>{
   if(revision===this.revision){this.set(items);this.errors.set('');}
  },error=>{
   if(revision===this.revision)this.errors.set(String(error));
   throw error;
  }).finally(()=>{this.refreshing=null;});
  return this.refreshing;
 };
 async toggle(id:string,enabled:boolean){
  const revision=++this.revision;this.pendingToggles++;
  try{const items=await this.invoke('set_plugin_enabled',{id,enabled});if(revision===this.revision)this.set(items);}
  finally{this.pendingToggles--;}
 }
 async toggleOptional(enabled:boolean){
  const revision=++this.revision;this.pendingToggles++;
  try{const items=await this.invoke('set_optional_plugins_enabled',{enabled});if(revision===this.revision)this.set(items);}
  finally{this.pendingToggles--;}
 }
 startPolling=()=>{
  this.stopPolling?.();
  let active=true;
  let timer:ReturnType<typeof setTimeout>;
  const poll=async()=>{
   // Refresh publishes transport errors without dropping the last known roster.
   try{await this.refresh();}catch{}
   if(active)timer=setTimeout(()=>void poll(),2000);
  };
  const stop=()=>{if(!active)return;active=false;clearTimeout(timer);this.revision++;};
  this.stopPolling=stop;
  void poll();
  return stop;
 };
 dispose=()=>{this.stopPolling?.();this.clear([]);this.errors.clear('');};
}
