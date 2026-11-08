import React,{useEffect,useRef,useState,useSyncExternalStore} from 'react';
import {MoreHorizontal} from 'lucide-react';
import type {ContextMenuTarget,ResolvedMenuAction,Scope} from '../../../../packages/plugin-sdk/src';
import {UiSlot,useUiPlugins} from '../app/plugins';
import {Button} from './ui/button';
import {DropdownMenu,DropdownMenuTrigger,DropdownMenuContent,DropdownMenuItem,DropdownMenuSeparator} from './ui/dropdown-menu';

/** The gesture captures its target; providers and rendering have separate lifetimes. */
export function ContextMenu({target,scope,label,disabled,children,legacy,className,triggerClassName,onDialogOpen}:{target:ContextMenuTarget;scope:Scope;label:string;disabled?:boolean;children:React.ReactNode;legacy?:(close:()=>void)=>React.ReactNode;className?:string;triggerClassName?:string;onDialogOpen?:boolean}){
 const registry=useUiPlugins().contextMenus;
 const revision=useSyncExternalStore(callback=>{const sub=registry.subscribe(callback);return ()=>sub.dispose();},registry.getSnapshot);
 const [captured,setCaptured]=useState<ContextMenuTarget|null>(null);
 const [items,setItems]=useState<ResolvedMenuAction[]>([]),[errors,setErrors]=useState<string[]>([]),[loading,setLoading]=useState(false);
 const [executionError,setExecutionError]=useState('');const executing=useRef(false),origin=useRef<HTMLElement|null>(null);
 const close=()=>setCaptured(null);
 const open=(element?:HTMLElement)=>{if(!disabled){origin.current=element??(document.activeElement instanceof HTMLElement?document.activeElement:null);setExecutionError('');setCaptured(structuredClone(target));}};
 const scopeId=JSON.stringify(scope);
 useEffect(()=>{
  if(!captured)return;
  const controller=new AbortController();setItems([]);setErrors([]);setLoading(true);
  void registry.resolve(JSON.parse(scopeId),captured,controller.signal).then(result=>{
   if(!controller.signal.aborted){setItems(result.actions);setErrors(result.errors);setLoading(false);}
  });
  return ()=>controller.abort();
 },[captured,revision,registry,scopeId]);
 async function run(item:ResolvedMenuAction){
  if(executing.current)return;executing.current=true;close();
  try{await item.run();}catch(error){setExecutionError(String(error));}finally{executing.current=false;}
 }
 return <>
  <DropdownMenu open={!!captured} onOpenChange={value=>value?open():close()}>
   <div className={className} onContextMenu={event=>{if(!event.defaultPrevented&&!disabled){event.preventDefault();open(event.target instanceof HTMLElement?event.target.closest('button')??event.currentTarget:event.currentTarget);}}} onKeyDown={event=>{
    if(!event.defaultPrevented&&!event.repeat&&!event.nativeEvent.isComposing&&(event.key==='ContextMenu'||event.key==='F10'&&event.shiftKey)){event.preventDefault();open(event.target as HTMLElement);}
   }}>
    {children}<DropdownMenuTrigger asChild><Button variant="ghost" size="icon-sm" className={triggerClassName} disabled={disabled} aria-label={label}><MoreHorizontal size={16}/></Button></DropdownMenuTrigger>
   </div>
   <DropdownMenuContent align="start" collisionPadding={8} onCloseAutoFocus={event=>{if(onDialogOpen)event.preventDefault();else if(origin.current?.isConnected){event.preventDefault();origin.current.focus({preventScroll:true});}}}>
    {loading&&<div role="status" className="ds-dropdown-label">Загрузка…</div>}
    {items.map((item,index)=><React.Fragment key={item.key}>
     {item.separatorBefore&&index>0&&<DropdownMenuSeparator/>}
     <DropdownMenuItem disabled={disabled||item.disabled} variant={item.danger?'destructive':'default'} onSelect={()=>void run(item)}>
      {item.icon&&<UiSlot slot="contextMenu.icon" scope={scope} entryId={item.icon.key} componentProps={{icon:item.icon}}/>}{item.label}
     </DropdownMenuItem>
    </React.Fragment>)}
    {captured&&legacy?.(close)}
    {errors.map(error=><p key={error} role="alert" className="workspace-error">{error}</p>)}
    {!loading&&!items.length&&!legacy&&!errors.length&&<div className="ds-dropdown-label">Нет доступных действий</div>}
   </DropdownMenuContent>
  </DropdownMenu>
  {executionError&&<p role="alert" className="workspace-error">{executionError}</p>}
 </>;
}
