import type {RunSnapshot,SessionInfo,SessionMessage} from '../../client/src/types.ts';
import type {SessionStream,StreamHandle} from '../../client/src/streams.ts';
export type SessionSnapshot={session:SessionInfo|null;messages:SessionMessage[];cursor:number;activeRun:RunSnapshot|null;status:'loading'|'ready'|'reconnecting'|'error';error:string|null};
const object=(v:unknown):v is Record<string,unknown>=>typeof v==='object'&&v!==null&&!Array.isArray(v);
const parts=(v:unknown)=>Array.isArray(v)&&v.every(p=>object(p)&&typeof p.kind==='string'&&Object.hasOwn(p,'data'));
const info=(v:unknown):v is SessionInfo=>object(v)&&typeof v.id==='string'&&typeof v.title==='string'&&typeof v.updatedAt==='number'&&Number.isFinite(v.updatedAt)&&(v.model===null||typeof v.model==='string');
const message=(v:unknown):v is SessionMessage=>object(v)&&typeof v.id==='string'&&(v.parentId===null||typeof v.parentId==='string')&&typeof v.role==='string'&&typeof v.createdAt==='string'&&parts(v.parts)&&object(v.extensions);
const run=(v:unknown):v is RunSnapshot=>object(v)&&typeof v.sessionId==='string'&&typeof v.runId==='string'&&['running','completed','failed','cancelled','interrupted'].includes(String(v.status))&&parts(v.parts)&&(v.error===null||typeof v.error==='string');
const empty=():SessionSnapshot=>({session:null,messages:[],cursor:0,activeRun:null,status:'loading',error:null});
/** A fresh baseline is authoritative. Events are accepted only within its sequence. */
export class SessionModel {
 private runs=new Map<string,RunSnapshot>();private state=empty();private listeners=new Set<()=>void>();private generation=0;private handle?:StreamHandle;private ready=false;private sessionId:string;private stream:SessionStream;
 constructor(sessionId:string,stream:SessionStream){this.sessionId=sessionId;this.stream=stream;}
 getSnapshot=()=>this.state;
 subscribe=(listener:()=>void)=>{this.listeners.add(listener);return ()=>{this.listeners.delete(listener)};};
 private set(value:SessionSnapshot){this.state=value;for(const listener of this.listeners)listener();}
 async follow():Promise<StreamHandle>{
  const epoch=++this.generation;this.handle?.dispose();this.handle=undefined;this.ready=false;
  const handle=await this.stream.follow(this.sessionId,frame=>{if(epoch===this.generation)this.receive(frame)});
  if(epoch!==this.generation){handle.dispose();return {dispose(){},reconnect(){}};}
  this.handle=handle;
  if(this.state.status==='error')handle.dispose();
  // A malformed synchronous baseline may have requested recovery before follow resolved.
  if(this.state.status==='reconnecting')handle.reconnect();
  return {dispose:()=>{if(epoch===this.generation)this.dispose()},reconnect:()=>{if(epoch===this.generation)this.resync()}};
 }
 private resync(){if(!this.ready&&this.state.status==='reconnecting')return;this.ready=false;this.set({...this.state,status:'reconnecting'});this.handle?.reconnect();}
 private overlay(messages:SessionMessage[],value:RunSnapshot){return messages.map(m=>m.id===value.runId?{...m,extensions:{...m.extensions,execution:{status:value.status,error:value.error}}}:m);}
 private receive(frame:unknown){
  if(!object(frame)||frame.sessionId!==this.sessionId)return;
  if(frame.kind==='disconnected'){this.ready=false;this.set({...this.state,status:'error',error:String(frame.error)});return;}
  if(frame.kind==='error'){this.ready=false;this.set({...this.state,status:'error',error:typeof frame.code==='string'?frame.code:'Stream failed'});this.handle?.dispose();return;}
  if(frame.kind==='resync'){this.ready=false;this.set({...this.state,status:'reconnecting'});return;}
  if(frame.kind==='baseline'){
   const b=frame.snapshot;
   if(!object(b)||!info(b.session)||b.session.id!==this.sessionId||!Array.isArray(b.messages)||!b.messages.every(message)||!Number.isSafeInteger(b.cursor)||Number(b.cursor)<0||!(b.activeRun===null||run(b.activeRun)&&b.activeRun.sessionId===this.sessionId)){this.resync();return;}
   this.runs.clear();if(b.activeRun)this.runs.set(b.activeRun.runId,b.activeRun);this.ready=true;this.set({session:b.session,messages:b.activeRun?this.overlay(b.messages,b.activeRun):b.messages,cursor:b.cursor as number,activeRun:b.activeRun,status:'ready',error:null});return;
  }
  if(frame.kind!=='event'||!this.ready)return;
  if(!Number.isSafeInteger(frame.seq)){this.resync();return;}
  const seq=frame.seq as number;if(seq<=this.state.cursor)return;if(seq!==this.state.cursor+1){this.resync();return;}
  const data=frame.data;let next={...this.state,cursor:seq};
  if(frame.eventType==='session.updated'&&info(data)&&data.id===this.sessionId)next.session=data;
  else if(frame.eventType==='message.upsert'&&message(data)){const index=next.messages.findIndex(m=>m.id===data.id);next.messages=index<0?[...next.messages,data]:next.messages.map((m,i)=>i===index?data:m);const execution=this.runs.get(data.id);if(execution)next.messages=this.overlay(next.messages,execution);}
  else if(frame.eventType==='run.updated'&&run(data)&&data.sessionId===this.sessionId){this.runs.set(data.runId,data);next.messages=this.overlay(next.messages,data);next.activeRun=data.status==='running'?data:null;}
  else {this.resync();return;}
  if(next.session)next.session={...next.session,hasMessages:next.messages.length>0};
  this.set(next);
 }
 dispose=()=>{this.generation++;this.ready=false;this.handle?.dispose();this.handle=undefined;this.runs.clear();this.set(empty());this.listeners.clear();};
}
