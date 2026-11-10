import {useState} from 'react';
import {Button} from '../../ui/button';

// The server owns the paused call; answering must not submit the tool again.
export function ToolApproval({prompt,onAnswer}:{prompt:string;onAnswer:(approved:boolean)=>Promise<void>}){
 const [busy,setBusy]=useState(false),[error,setError]=useState('');
 async function answer(approved:boolean){setBusy(true);setError('');try{await onAnswer(approved);}catch{setError('Ответ не принят. Проверьте подключение и повторите.');}finally{setBusy(false);}}
 return <section className="harness-interaction" aria-label="Разрешение на выполнение"><strong>Требуется разрешение</strong><p>{prompt}</p><div className="provider-actions"><Button type="button" disabled={busy} onClick={()=>void answer(false)}>Отклонить</Button><Button type="button" disabled={busy} onClick={()=>void answer(true)}>Разрешить этот вызов</Button></div>{error&&<p role="alert">{error}</p>}</section>;
}
