import {scopeKey,type Scope,type Disposable} from './lifecycle.ts';

/** Features extend this map through declaration merging. */
export interface ContextMenuTargetMap {}
export type ContextMenuTarget = {[K in keyof ContextMenuTargetMap]:{kind:K;value:ContextMenuTargetMap[K]}}[keyof ContextMenuTargetMap];
export interface ContextMenuAction {
 id:string; label:string; icon?:{key:string;src?:string};
 disabled?:boolean; danger?:boolean; separatorBefore?:boolean;
 run:()=>void|Promise<void>;
}
export interface ContextMenuContribution<K extends keyof ContextMenuTargetMap> {
 id:string; target:K; order?:number;
 resolve:(value:ContextMenuTargetMap[K],signal:AbortSignal)=>readonly ContextMenuAction[]|Promise<readonly ContextMenuAction[]>;
}
type RecordEntry={owner:string;scope:string;contribution:ContextMenuContribution<any>;alive:boolean;pending:Set<AbortController>};
export type ResolvedMenuAction=ContextMenuAction & {key:string;ownerPluginId:string};
export class ContextMenuRegistry {
 private records=new Set<RecordEntry>();private listeners=new Set<()=>void>();private revision=0;
 getSnapshot=()=>this.revision;
 subscribe=(listener:()=>void):Disposable=>{this.listeners.add(listener);return {dispose:()=>{this.listeners.delete(listener);}};};
 private changed(){this.revision++;for(const listener of this.listeners){try{listener();}catch{/* Observers cannot break plugin cleanup. */}}}
 register<K extends keyof ContextMenuTargetMap>(scope:Scope,owner:string,contribution:ContextMenuContribution<K>):Disposable {
  const key=scopeKey(scope);
  if(!owner||!contribution.id||!contribution.target||typeof contribution.resolve!=='function'||!Number.isFinite(contribution.order??0))throw Error('Invalid menu contribution');
  if([...this.records].some(r=>r.scope===key&&r.contribution.id===contribution.id))throw Error('Duplicate menu contribution');
  const record:RecordEntry={owner,scope:key,contribution:{...contribution},alive:true,pending:new Set()};
  this.records.add(record);this.changed();
  return {dispose:()=>{if(!record.alive)return;record.alive=false;this.records.delete(record);for(const controller of record.pending)controller.abort();this.changed();}};
 }
 async resolve(scope:Scope,target:ContextMenuTarget,signal:AbortSignal):Promise<{actions:ResolvedMenuAction[];errors:string[]}> {
  const records=[...this.records].filter(r=>r.scope===scopeKey(scope)&&r.contribution.target===target.kind)
   .sort((a,b)=>(a.contribution.order??0)-(b.contribution.order??0)||(a.contribution.id<b.contribution.id?-1:1));
  const results=await Promise.all(records.map(async record=>{
   const controller=new AbortController();record.pending.add(controller);
   const abort=()=>controller.abort();signal.addEventListener('abort',abort,{once:true});
   if(signal.aborted)abort();
   let timer:ReturnType<typeof setTimeout>|undefined;
   try{
    const interrupted=new Promise<never>((_,reject)=>{
     const stop=()=>reject(controller.signal.reason??Error('Menu preparation cancelled'));
     controller.signal.addEventListener('abort',stop,{once:true});
     if(controller.signal.aborted)stop();
     timer=setTimeout(()=>controller.abort(Error('Menu preparation timed out')),1000);
    });
    const items=await Promise.race([interrupted,Promise.resolve().then(()=>{
     controller.signal.throwIfAborted();return record.contribution.resolve(structuredClone(target.value),controller.signal);
    })]);
    const ids=new Set<string>();
    const actions=items.map(item=>{
     if(typeof item.id!=='string'||!item.id||ids.has(item.id)||typeof item.label!=='string'||!item.label||typeof item.run!=='function'||item.icon&&(typeof item.icon.key!=='string'||!item.icon.key))throw Error('Invalid menu action');
     ids.add(item.id);
     const run=item.run,disabled=item.disabled;
     return {...item,key:JSON.stringify([record.owner,record.contribution.id,item.id]),ownerPluginId:record.owner,run:async()=>{
      if(!record.alive||disabled)throw Error('Menu action is unavailable');
      await run();
     }};
    });
    return {actions:record.alive&&!signal.aborted?actions:[],errors:[] as string[]};
   }catch(error){return {actions:[] as ResolvedMenuAction[],errors:signal.aborted||!record.alive?[]:[`${record.owner}: ${String(error)}`]};}
   finally{clearTimeout(timer);signal.removeEventListener('abort',abort);record.pending.delete(controller);}
  }));
  if(signal.aborted)return {actions:[],errors:[]};
  const current=results.filter((_,index)=>records[index].alive);
  return {actions:current.flatMap(r=>r.actions),errors:current.flatMap(r=>r.errors)};
 }
}
