import {useEffect,useRef,useState} from 'react';
import type {ImageMessagePartProps} from '@assistant-ui/react';
import {ExecutionImage} from '../execution/ExecutionImage';
import {invoke} from '../client';
export function SessionImage({sessionId,...props}:ImageMessagePartProps&{sessionId:string}){
 const attachment=(props.providerMetadata?.minerva as {attachment?:{id?:string}}|undefined)?.attachment;
 const key=`${sessionId}/${attachment?.id??''}`;
 const [loaded,setLoaded]=useState({key,image:props.image});const [failed,setFailed]=useState('');const target=useRef<HTMLDivElement>(null);
 useEffect(()=>{let disposed=false;setLoaded({key,image:props.image});setFailed('');
  if(props.image||!attachment?.id)return;
  const observer=new IntersectionObserver(entries=>{if(!entries.some(entry=>entry.isIntersecting))return;observer.disconnect();
   void invoke('attachments.get',{sessionId,id:attachment.id!}).then(value=>{if(!disposed)setLoaded({key,image:`data:${value.mimeType};base64,${value.data}`});}).catch(()=>{if(!disposed)setFailed(key);});
  },{rootMargin:'200px'});
  if(target.current)observer.observe(target.current);
  return ()=>{disposed=true;observer.disconnect();};
 },[sessionId,attachment?.id,props.image]);
 return <div ref={target}>{failed===key?<p role="alert">Изображение недоступно. Откройте сессию заново для повторной загрузки.</p>:<ExecutionImage {...props} image={loaded.key===key?loaded.image:''}/>}</div>;
}
