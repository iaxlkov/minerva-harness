// Adapted to Minerva's styles from assistant-ui's image element.
import {useRef,useState} from 'react';
import type {ImageMessagePartProps} from '@assistant-ui/react';
import {X} from 'lucide-react';

export function ImageGenerating({awaitingApproval=false,callId}:{awaitingApproval?:boolean;callId:string}){
 return <div className="image-generating" data-call-id={callId} data-tool-state={awaitingApproval?'requires-approval':'running'} role="status" aria-label={awaitingApproval?'Ожидание разрешения':'Создание изображения'}>
  <span>{awaitingApproval?'Ожидание разрешения':'Создаю изображение'}</span>
 </div>;
}

export function Image({image,filename,status}:ImageMessagePartProps){
 const [failedImage,setFailedImage]=useState(''),[zoomed,setZoomed]=useState(false),[copyError,setCopyError]=useState('');
 const dialog=useRef<HTMLDialogElement>(null);
 async function copy(){
  setCopyError('');
  try{
   const blob=await (await fetch(image)).blob();
   let png=blob;
   if(blob.type!=='image/png'){
    const bitmap=await createImageBitmap(blob);
    try{const canvas=document.createElement('canvas');canvas.width=bitmap.width;canvas.height=bitmap.height;const ctx=canvas.getContext('2d');if(!ctx)throw Error('Canvas unavailable');ctx.drawImage(bitmap,0,0);png=await new Promise<Blob>((resolve,reject)=>canvas.toBlob(value=>value?resolve(value):reject(Error('PNG conversion failed')),'image/png'));}finally{bitmap.close();}
   }
   await navigator.clipboard.write([new ClipboardItem({'image/png':png})]);
  }catch{setCopyError('Не удалось скопировать изображение.');}
 }
 if(status?.type==='incomplete'&&status.reason==='content-filter')return <p role="alert">Изображение отклонено фильтром содержимого.</p>;
 if(!image||failedImage===image)return <p role="status">{image&&failedImage===image?'Не удалось показать изображение.':'Изображение загружается…'}</p>;
 return <figure className="execution-image">
  <button type="button" className="execution-image-preview" aria-label="Увеличить изображение" onClick={()=>{setZoomed(true);dialog.current?.showModal();}}><img src={image} alt={filename||'Изображение'} onError={()=>setFailedImage(image)}/></button>
  <dialog ref={dialog} className="execution-image-dialog" aria-label="Изображение" onClose={()=>setZoomed(false)}><button type="button" className="execution-image-close" aria-label="Закрыть просмотр" onClick={()=>dialog.current?.close()}><X size={22} aria-hidden="true"/></button>{zoomed&&<img src={image} alt={filename||'Изображение'}/>}</dialog>
  {filename&&<figcaption>{filename}</figcaption>}
  <div className="execution-image-actions"><a href={image} download={filename||'image.png'}>Скачать</a><button type="button" onClick={()=>void copy()}>Копировать</button></div>
  {copyError&&<p role="alert">{copyError}</p>}
 </figure>;
}
