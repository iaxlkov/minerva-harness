import type {ComponentProps} from 'react';
import {ComposerPrimitive, QueueItemPrimitive, AuiIf, useAui} from '@assistant-ui/react';
import {FileText, CornerDownRight, X} from 'lucide-react';
import type {BrowserPlugin, Scope} from '../../../packages/plugin-sdk/src/index';
import {Button} from '../../../apps/web/src/components/ui/button';
import './queue.css';

export const messageQueueOptions = {
 unstable_enableMessageQueue:true,
 unstable_queueClearOnCancel:false,
 unstable_queueClearOnRewind:false,
};
// Explicit FIFO also applies while a previously queued message is running.
export function QueueComposerRoot(props:ComponentProps<typeof ComposerPrimitive.Root>){
 const aui=useAui();
 return <ComposerPrimitive.Root {...props} onSubmit={event=>{props.onSubmit?.(event);if(!event.defaultPrevented){event.preventDefault();aui.composer().send({steer:false});}}}/>;
}
export function QueueComposerSend(props:ComponentProps<typeof ComposerPrimitive.Send>){
 const aui=useAui();
 return <ComposerPrimitive.Send {...props} onClick={event=>{props.onClick?.(event);if(!event.defaultPrevented){event.preventDefault();aui.composer().send({steer:false});}}}/>;
}
function MessageQueue(){
 return <AuiIf condition={s=>s.composer.queue.length>0}><section className="message-queue" aria-label="Очередь сообщений">
  <div className="message-queue-heading">Очередь сообщений <AuiIf condition={s=>!s.thread.isRunning}><span>· Приостановлена — отправьте сообщение для продолжения</span></AuiIf></div>
  <ComposerPrimitive.Queue>{({queueItem})=><div className="message-queue-item">
   <CornerDownRight size={14} aria-hidden="true"/>
   <div className="message-queue-content"><QueueItemPrimitive.Text/>
    {queueItem.parts.filter(part=>part.type==='file').map((part,index)=><span className="message-queue-file" key={index}><FileText size={14} aria-hidden="true"/>{part.filename||'Изображение'}</span>)}
   </div>
   <QueueItemPrimitive.Steer asChild><Button type="button" variant="ghost" size="sm" title="Обработать следующим после текущего ответа">Следующим</Button></QueueItemPrimitive.Steer>
   <QueueItemPrimitive.Remove asChild><Button type="button" variant="ghost" size="sm" aria-label="Удалить из очереди"><X size={14}/></Button></QueueItemPrimitive.Remove>
  </div>}</ComposerPrimitive.Queue>
 </section></AuiIf>;
}
// shortcut: pending messages live in the mounted chat; add server persistence for background delivery.
export function messageQueuePlugin(scope:Scope):BrowserPlugin{return {id:'message-queue-ui',requires:['minerva-ui'],apply(ctx){ctx.scope(scope).slots.register('composer.queue',{entryId:'default',component:MessageQueue});}};}
