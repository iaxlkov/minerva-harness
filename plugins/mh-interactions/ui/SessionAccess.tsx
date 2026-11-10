import {useEffect,useState} from 'react';
import {Shield} from 'lucide-react';
import type {WorkspaceSettings} from '../../../packages/client/src/types';
import {invoke} from '../../../apps/web/src/client';
import {Select,SelectTrigger,SelectValue,SelectContent,SelectItem} from '../../../apps/web/src/components/ui/select';

export function SessionAccess({sessionId,disabled=false}:{sessionId:string;disabled?:boolean}){
 const [policy,setPolicy]=useState<WorkspaceSettings['permissionPolicy']>(),[busy,setBusy]=useState(false),[error,setError]=useState('');
 useEffect(()=>{let disposed=false;void invoke('workspace.get',{sessionId}).then(settings=>{if(!disposed)setPolicy(settings.permissionPolicy);}).catch(()=>{if(!disposed)setError('Не удалось загрузить доступ.');});return()=>{disposed=true;};},[sessionId]);
 async function save(value:WorkspaceSettings['permissionPolicy']){setBusy(true);setError('');try{const settings=await invoke('workspace.get',{sessionId});const saved=await invoke('workspace.save',{sessionId,settings:{...settings,permissionPolicy:value}});setPolicy(saved.permissionPolicy);}catch{setError('Не удалось сохранить доступ.');}finally{setBusy(false);}}
 return <div className="session-access"><Select value={policy} disabled={disabled||busy||!policy} onValueChange={value=>void save(value as WorkspaceSettings['permissionPolicy'])}><SelectTrigger aria-label="Доступ сессии" className="session-access-trigger"><Shield size={14} aria-hidden="true"/><SelectValue placeholder="Загрузка доступа…"/></SelectTrigger><SelectContent><SelectItem value="read-only">Только чтение</SelectItem><SelectItem value="ask">С подтверждением</SelectItem><SelectItem value="allow">Полный доступ</SelectItem></SelectContent></Select>{error&&<span role="alert">{error}</span>}</div>;
}
