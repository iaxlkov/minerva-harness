import {useState} from 'react';
import type {ImageMessagePartProps} from '@assistant-ui/react';
import {object} from './toolModel';
import './execution.css';

export function ExecutionImage({image,providerMetadata}:ImageMessagePartProps) {
  const [failed,setFailed]=useState(false);
  const attachment=object(object(providerMetadata).minerva).attachment;
  const name=object(attachment).name;
  return <figure className="execution-image">
    {image&&!failed?<img src={image} alt={typeof name==='string'?name:'Прикреплённое изображение'} onError={()=>setFailed(true)}/>:<p role="status">{failed?'Не удалось показать изображение.':'Изображение загружается…'}</p>}
    {typeof name==='string'&&<figcaption>{name}</figcaption>}
  </figure>;
}
