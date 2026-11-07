/** Serial reconnect polling; disposal fences all late results. */
export function pollSession<T>(read:()=>Promise<T>,receive:(value:T)=>void,failed:()=>void,delay=1200):()=>void {
 let disposed=false,timer:ReturnType<typeof setTimeout>|undefined;
 const tick=async()=>{try{const value=await read();if(!disposed)receive(value);}catch{if(!disposed)failed();}finally{if(!disposed)timer=setTimeout(tick,delay);}};
 void tick();return ()=>{disposed=true;clearTimeout(timer);};
}
