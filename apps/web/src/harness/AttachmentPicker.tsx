import {useCallback,useEffect,useId,useRef,useState,type ReactNode} from 'react';
import {useAui,useAuiEvent} from '@assistant-ui/react';
import {Paperclip,Plus} from 'lucide-react';
import {Button} from '../components/ui/button';
import {ComposerMenu,ComposerMenuItem} from '../components/assistant-ui/composer-menu';
export function AttachmentPicker({disabled=false,renderMenu}:{disabled?:boolean;renderMenu?:(onClose:()=>void)=>ReactNode}){
 const aui=useAui(),input=useRef<HTMLInputElement>(null);
 const [error,setError]=useState(''),[open,setOpen]=useState(false);
 const trigger=useRef<HTMLButtonElement>(null),menuId=useId();
 const close=useCallback(()=>{setOpen(false);trigger.current?.focus();},[]);
 useEffect(()=>{if(disabled)setOpen(false);},[disabled]);
 const failed=()=>setError('Не удалось добавить файл. Максимальный размер — 10 МиБ.');
 useAuiEvent('composer.attachmentAddError',failed);
 return <div className="composer-add">
  <input ref={input} hidden type="file" multiple disabled={disabled} onChange={event=>{
   const files=Array.from(event.target.files??[]);event.target.value='';setError('');
   for(const file of files)void aui.composer().addAttachment(file).catch(failed);
  }}/>
  <Button ref={trigger} type="button" variant="ghost" size="icon" disabled={disabled} aria-label="Добавить вложение" aria-haspopup="menu" aria-expanded={open&&!disabled} aria-controls={open&&!disabled?menuId:undefined} className="composer-add-trigger" onClick={()=>setOpen(value=>!value)} onKeyDown={event=>{if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();setOpen(true);}}}><Plus size={18}/></Button>
  <ComposerMenu id={menuId} aria-label="Вложения и действия" open={open&&!disabled} onClose={close} triggerRef={trigger}>
   <ComposerMenuItem disabled={disabled} onClick={()=>{setError('');input.current?.click();}}><Paperclip size={16}/> Прикрепить файл</ComposerMenuItem>
   {renderMenu?.(close)}
  </ComposerMenu>
  {error&&<p role="alert" className="composer-attachment-error">{error}</p>}
 </div>;
}
