import {useRef,useState} from 'react';
import {useAui,useAuiEvent} from '@assistant-ui/react';
import {Button} from '../components/ui/button';
export function AttachmentPicker({disabled=false}:{disabled?:boolean}){
 const aui=useAui(),input=useRef<HTMLInputElement>(null);const [error,setError]=useState('');
 useAuiEvent('composer.attachmentAddError',()=>setError('Не удалось добавить изображение. Используйте PNG, JPEG, GIF или WebP до 10 МиБ.'));
 return <><input ref={input} hidden type="file" multiple accept="image/png,image/jpeg,image/gif,image/webp" onChange={event=>{const files=Array.from(event.target.files??[]);event.target.value='';setError('');for(const file of files)void aui.composer().addAttachment(file).catch(()=>setError('Не удалось добавить изображение. Используйте PNG, JPEG, GIF или WebP до 10 МиБ.'));}}/><Button type="button" variant="ghost" size="sm" disabled={disabled} onClick={()=>input.current?.click()}>Прикрепить изображение</Button>{error&&<p role="alert">{error}</p>}</>;
}
