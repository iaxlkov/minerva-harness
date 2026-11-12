import { ScrollArea } from '../../../apps/web/src/components/ui/scroll-area';
import {useEffect,useRef,useState} from 'react';
import type {ContextBudget,PendingInteraction} from '../../../packages/client/src/types';
import {invoke} from '../../../apps/web/src/client';
import {Button} from '../../../apps/web/src/components/ui/button';
import {pollSession} from '../../../apps/web/src/harness/poll';
import '../../../apps/web/src/harness/harness.css';
export function SessionControls({sessionId,inlineApprovals=false}:{sessionId:string;inlineApprovals?:boolean}){
 const answered=useRef(new Set<string>());
 const [pending,setPending]=useState<PendingInteraction[]>([]),[budget,setBudget]=useState<ContextBudget|null>(null),[offline,setOffline]=useState(false),[error,setError]=useState(''),[busy,setBusy]=useState(false);
 useEffect(()=>{answered.current.clear();setPending([]);setBudget(null);setError('');return pollSession(async()=>Promise.all([invoke('interactions.list',{sessionId}),invoke('context.status',{sessionId})]),([items,status])=>{setPending(items.filter(item=>item.sessionId===sessionId&&!answered.current.has(item.id)));setBudget(status);setOffline(false);},()=>setOffline(true));},[sessionId]);
 async function answer(item:PendingInteraction,decision:string,value:string|null=null){setBusy(true);setError('');try{const accepted=await invoke('interactions.answer',{sessionId:item.sessionId,id:item.id,decision,answer:value});if(!accepted)throw Error('Срок действия запроса истёк.');answered.current.add(item.id);setPending(items=>items.filter(entry=>entry.id!==item.id));}catch{setError('Ответ не принят. После подключения проверьте, ожидает ли запрос ответа.');}finally{setBusy(false);}}
 const shown=pending.filter(item=>!inlineApprovals||item.kind!=='approval');
 if(!shown.length&&!offline&&!error&&!budget?.usage)return null;
 return <ScrollArea asChild><section className="harness-session" aria-label="Управление сессией">
  {budget?.usage&&<p>Расход за последний ход: {budget.usage.inputTokens??'нет данных'} входящих / {budget.usage.outputTokens??'нет данных'} исходящих / {budget.usage.totalTokens??'нет данных'} токенов всего</p>}
  {offline&&<p role="status">Переподключение к сессии… Запросы ожидают ответа на сервере.</p>}
  {error&&<p role="alert">{error}</p>}
  {shown.map(item=><article key={item.id} className="harness-interaction" data-interaction-id={item.id}><strong>{item.title}</strong><p>{item.prompt}</p><small>Ход {item.runId} · Вызов {item.callId}</small>{item.kind==='approval'?<div className="provider-actions"><Button disabled={busy||offline} onClick={()=>void answer(item,'deny')}>Отклонить</Button><Button disabled={busy||offline} onClick={()=>void answer(item,'allow')}>Разрешить этот вызов</Button></div>:<Question key={item.id} item={item} disabled={busy||offline} onAnswer={value=>void answer(item,'answer',value)}/>}</article>)}
 </section></ScrollArea>;
}
function Question({item,disabled,onAnswer}:{item:PendingInteraction;disabled:boolean;onAnswer:(value:string)=>void}){const [value,setValue]=useState('');return <form onSubmit={event=>{event.preventDefault();if(value.trim())onAnswer(value);}}>{item.options.length>0?<fieldset disabled={disabled}><legend>Выберите ответ</legend>{item.options.map(option=><label key={option}><input type="radio" name={item.id} value={option} checked={value===option} onChange={()=>setValue(option)}/>{option}</label>)}</fieldset>:<label>Ответ<input required disabled={disabled} value={value} onChange={event=>setValue(event.target.value)}/></label>}<Button type="submit" disabled={disabled||!value.trim()}>Отправить ответ</Button></form>;}
