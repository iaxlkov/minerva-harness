import {useState} from 'react';
import type {FileMessagePartProps} from '@assistant-ui/react';
import {FileDown} from 'lucide-react';
import {Button} from '../components/ui/button';
import {invoke} from '../client';
import type {AttachmentRef} from '../../../../packages/client/src/types';
export function SessionFile({sessionId,filename,providerMetadata}:FileMessagePartProps&{sessionId:string}){
 const [error,setError]=useState(''),[busy,setBusy]=useState(false);
 const ref=(providerMetadata?.minerva as {attachment?:AttachmentRef}|undefined)?.attachment;
 async function download(){
  if(!ref)return;
  setBusy(true);setError('');
  try{
   const file=await invoke('attachments.get',{sessionId,id:ref.id});
   const bytes=Uint8Array.from(atob(file.data),char=>char.charCodeAt(0));
   const url=URL.createObjectURL(new Blob([bytes],{type:'application/octet-stream'}));
   const link=document.createElement('a');link.href=url;link.download=file.name;link.click();
   setTimeout(()=>URL.revokeObjectURL(url),1000);
  }catch{setError('Не удалось скачать файл. Попробуйте ещё раз.');}
  finally{setBusy(false);}
 }
 return <div className="session-file"><Button type="button" variant="outline" disabled={busy||!ref} onClick={()=>void download()}><FileDown size={16}/>{filename||ref?.name||'Файл'}{ref&&<small>{Math.ceil(ref.bytes/1024)} КиБ</small>}</Button>{error&&<p role="alert">{error}</p>}</div>;
}
