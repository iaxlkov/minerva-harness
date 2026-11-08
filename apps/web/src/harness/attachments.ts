import type {AttachmentAdapter} from '@assistant-ui/react';
import type {Invoke} from '../../../../packages/client/src/types.ts';
const raster=['image/png','image/jpeg','image/gif','image/webp'];
export function attachmentAdapter(sessionId:string,invoke:Invoke):AttachmentAdapter {
 return {accept:'*',async add({file}){
  if(file.size>10*1024*1024)throw Error('Размер файла не должен превышать 10 МиБ.');
  return {id:crypto.getRandomValues(new Uint32Array(4)).join('-'),type:raster.includes(file.type)?'image':'file',name:file.name,contentType:file.type||'application/octet-stream',file,status:{type:'requires-action',reason:'composer-send'}};
 },async send(attachment){
  const bytes=new Uint8Array(await attachment.file.arrayBuffer());let binary='';
  for(let offset=0;offset<bytes.length;offset+=32768)binary+=String.fromCharCode(...bytes.subarray(offset,offset+32768));
  const ref=await invoke('attachments.put',{sessionId,name:attachment.name,mimeType:attachment.contentType!,data:btoa(binary)});
  return {...attachment,status:{type:'complete'},content:[attachment.type==='image'?{type:'image',image:'',providerMetadata:{minerva:{attachment:ref}}}:{type:'file',data:'',mimeType:ref.mimeType,filename:ref.name,providerMetadata:{minerva:{attachment:ref}}}]};
 },async remove(){}};
}
