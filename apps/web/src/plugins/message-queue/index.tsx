import {ComposerPrimitive, QueueItemPrimitive, AuiIf} from '@assistant-ui/react';
import {FileText, CornerDownRight, X} from 'lucide-react';
import type {BrowserPlugin, Scope} from '../../../../../packages/plugin-sdk/src/index';
import {Button} from '../../components/ui/button';
import './queue.css';

export const messageQueueOptions = {
 unstable_enableMessageQueue:true,
 unstable_queueClearOnCancel:false,
 unstable_queueClearOnRewind:false,
};
function MessageQueue(){
 return <AuiIf condition={s=>s.composer.queue.length>0}><section className="message-queue" aria-label="Очередь сообщений">
  <ComposerPrimitive.Queue>{({queueItem})=><div className="message-queue-item">
   <CornerDownRight size={14} aria-hidden="true"/>
   <div className="message-queue-content"><QueueItemPrimitive.Text/>
    {queueItem.parts.filter(part=>part.type==='file').map((part,index)=><span className="message-queue-file" key={index}><FileText size={14} aria-hidden="true"/>{part.filename||'Изображение'}</span>)}
   </div>
   <QueueItemPrimitive.Steer asChild><Button type="button" variant="ghost" size="sm">Следующим</Button></QueueItemPrimitive.Steer>
   <QueueItemPrimitive.Remove asChild><Button type="button" variant="ghost" size="sm" aria-label="Удалить из очереди"><X size={14}/></Button></QueueItemPrimitive.Remove>
  </div>}</ComposerPrimitive.Queue>
 </section></AuiIf>;
}
// shortcut: pending messages live in the mounted chat; add server persistence for background delivery.
export function messageQueuePlugin(scope:Scope):BrowserPlugin{return {id:'message-queue-ui',requires:['minerva-ui'],apply(ctx){ctx.scope(scope).slots.register('composer.queue',{entryId:'default',component:MessageQueue});}};}
