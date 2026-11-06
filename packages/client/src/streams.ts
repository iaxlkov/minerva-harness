/** Session subscriptions own observation only: closing one never cancels a run. */
export type StreamHandle={dispose():void;reconnect():void};
export type SessionStream={follow(sessionId:string,receive:(frame:unknown)=>void):Promise<StreamHandle>};
export type StreamSocket={send(data:string):void;close():void;onopen:((event:unknown)=>void)|null;onmessage:((event:{data:unknown})=>void)|null;onclose:((event:{code:number})=>void)|null;onerror:((event:unknown)=>void)|null};
export type StreamOptions={url?:string;workspaceId?:string;webSocketFactory?:(url:string)=>StreamSocket;setTimeout?:(fn:()=>void,ms:number)=>unknown;clearTimeout?:(timer:unknown)=>void;maxRetries?:number};
export function createSessionStream(options:StreamOptions={}):SessionStream {
 const factory=options.webSocketFactory??((url:string)=>new WebSocket(url) as unknown as StreamSocket);
 const schedule=options.setTimeout??((fn,ms)=>globalThis.setTimeout(fn,ms));
 const cancel=options.clearTimeout??((timer)=>globalThis.clearTimeout(timer as ReturnType<typeof setTimeout>));
 return {async follow(sessionId,receive){
  const url=new URL(options.url??'/api/v1/streams',globalThis.location?.href??'http://localhost');url.protocol=url.protocol==='https:'||url.protocol==='wss:'?'wss:':'ws:';
  if(options.workspaceId)url.searchParams.set('workspaceId',options.workspaceId);
  const id=crypto.randomUUID();let socket:StreamSocket|undefined,timer:unknown,generation=0,disposed=false,retries=0;
  function connect(){
   if(disposed)return;const epoch=++generation;
   const valid=()=>!disposed&&epoch===generation;
   function retry(){if(!valid())return;generation++;if(retries>=(options.maxRetries??8)){receive({kind:'disconnected',sessionId,error:'Stream retry limit reached'});return;}timer=schedule(()=>{timer=undefined;connect()},Math.min(30000,500*2**retries++));}
   try{socket=factory(url.toString());}catch{retry();return;}
   const current=socket;
   current.onopen=()=>{if(valid())current.send(JSON.stringify({kind:'open',id,sessionId}));};
   current.onmessage=event=>{if(!valid())return;let frame;try{frame=JSON.parse(String(event.data));}catch{return;}
    if(frame?.id!==id||frame?.sessionId!==sessionId)return;
    if(frame.kind==='baseline')retries=0;receive(frame);
   };
   current.onclose=event=>{if(!valid())return;if([1008,4001,4003,4401,4403].includes(event.code)){generation++;receive({kind:'disconnected',sessionId,error:'Authentication required'});return;}retry();};
   current.onerror=()=>{/* Browser error events are followed by close, which owns retry. */};
  }
  const handle:StreamHandle={dispose(){if(disposed)return;disposed=true;generation++;if(timer!==undefined)cancel(timer);try{socket?.send(JSON.stringify({kind:'close',id}));}catch{}socket?.close();},reconnect(){if(disposed)return;generation++;if(timer!==undefined){cancel(timer);timer=undefined;}socket?.close();connect();}};
  connect();return handle;
 }};
}
