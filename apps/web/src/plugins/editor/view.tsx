import React,{useEffect,useRef,useState,useSyncExternalStore} from 'react';
import {ChevronRight,File,FileImage,FileText,Folder,FolderOpen,X} from 'lucide-react';
import {Button} from '../../components/ui/button';
import {ScrollArea} from '../../components/ui/scroll-area';
import {Sheet,SheetContent,SheetHeader,SheetTitle,SheetDescription,SheetFooter} from '../../components/ui/sheet';
import type {Doc} from './store';
import {invoke} from '../../client';
import {models} from '../../app/models';
import {UiSlot,workspaceScope} from '../../app/plugins';
import {open,close,save,state,subscribe,snapshot,changed,registerLanguage,monaco,textOf} from './store';
import './editor.css';
type Entry={name:string;path:string;directory:boolean};
function Tree({view,sessionId,path='',depth=0}:{view:string;sessionId:string;path?:string;depth?:number}){
 const [items,setItems]=useState<Entry[]|null>(null);const [error,setError]=useState('');
 useEffect(()=>{let active=true;setItems(null);setError('');void invoke('mh-editor.list',{sessionId,path}).then(v=>{if(active)setItems(v);}).catch(e=>{if(active)setError(String(e));});return()=>{active=false;};},[sessionId,path]);
 if(error)return <p className="editor-error" role="alert">{error}</p>;
 if(!items)return <p className="editor-hint" role="status">Загрузка…</p>;
 if(items.length===0)return <p className="editor-hint">Папка пуста</p>;
 return <>{items.map(item=><TreeItem key={item.path} view={view} sessionId={sessionId} item={item} depth={depth}/>)}</>;
}
function TreeItem({view,sessionId,item,depth}:{view:string;sessionId:string;item:Entry;depth:number}){
 const [expanded,setExpanded]=useState(false);
 return <><button className="editor-file" title={item.path} style={{paddingLeft:8+depth*14}} onClick={()=>item.directory?setExpanded(v=>!v):void open(view,item.path,false,sessionId)} onDoubleClick={()=>{if(!item.directory)void open(view,item.path,true,sessionId);}}>{item.directory?<ChevronRight size={13} className={expanded?'expanded':''}/>:<span className="editor-file-spacer"/>}{item.directory?(expanded?<FolderOpen size={15}/>:<Folder size={15}/>):<File size={15}/>}<span>{item.name}</span></button>{item.directory&&expanded&&<Tree view={view} sessionId={sessionId} path={item.path} depth={depth+1}/>}</>;
}
function Editor({sessionId}:{sessionId:string}){
 useSyncExternalStore(subscribe,snapshot,snapshot);const s=state(sessionId);const doc=s.docs.find(d=>d.key===s.active);const element=useRef<HTMLDivElement>(null);const editor=useRef<monaco.editor.IStandaloneCodeEditor|null>(null);const [closing,setClosing]=useState<Doc|null>(null);
 useEffect(()=>{if(!element.current)return;const instance=monaco.editor.create(element.current,{theme:'vs-dark',automaticLayout:true,minimap:{enabled:false},fontSize:13,scrollBeyondLastLine:false,renderLineHighlight:'line',tabSize:2,readOnlyMessage:{value:'Файл открыт только для чтения'}});editor.current=instance;return()=>{editor.current=null;instance.dispose();};},[]);
 useEffect(()=>{if(!editor.current)return;editor.current.updateOptions({readOnly:!!doc?.readOnlyReason});editor.current.setModel(doc?.model??null);if(doc){registerLanguage(doc.model.getLanguageId());editor.current.focus();}},[doc]);
 useEffect(()=>{if(!doc?.lsp)return;let active=true;async function poll(){try{const all=await invoke('mh-lsp.diagnostics',{sessionId:doc!.sessionId,path:doc!.path}) as any[];if(!active)return;const diag=all[0]?.diagnostics??[];monaco.editor.setModelMarkers(doc!.model,'mh-lsp',diag.map((d:any)=>({message:d.message,severity:d.severity===1?monaco.MarkerSeverity.Error:d.severity===2?monaco.MarkerSeverity.Warning:monaco.MarkerSeverity.Info,startLineNumber:d.range.start.line+1,startColumn:d.range.start.character+1,endLineNumber:d.range.end.line+1,endColumn:d.range.end.character+1})));}catch{} }void poll();const timer=setInterval(()=>void poll(),2000);return()=>{active=false;clearInterval(timer);};},[sessionId,doc,doc?.lsp]);
 useEffect(()=>{const key=(event:KeyboardEvent)=>{if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='s'&&doc&&!doc.preview){event.preventDefault();void save(doc);}};window.addEventListener('keydown',key);return()=>window.removeEventListener('keydown',key);},[doc]);
 return <div className="editor-workbench"><div className="editor-tabs"><UiSlot slot="editor.tabs.start" scope={workspaceScope} componentProps={{sessionId,activePath:doc?.path??null}}/><div className="editor-tabs-list" role="tablist" aria-label="Открытые файлы">{s.docs.map(d=><div className={`editor-tab ${d===doc?'active':''} ${textOf(d)!==d.saved?'dirty':''} ${d.transient?'transient':''}`} key={d.key}><button role="tab" aria-selected={d===doc} title={d.transient?`${d.path} — предпросмотр, двойной щелчок закрепит вкладку`:d.path} onClick={()=>{s.active=d.key;changed();}} onDoubleClick={()=>{d.transient=false;changed();}}>{d.preview?.mime==='application/pdf'?<FileText size={13} aria-hidden="true"/>:d.preview?<FileImage size={13} aria-hidden="true"/>:<File size={13} aria-hidden="true"/>}<span className="editor-tab-name">{d.path.split('/').pop()}</span>{textOf(d)!==d.saved&&<><span className="editor-dirty" aria-hidden="true"/><span className="oe-sr-only">, не сохранён</span></>}</button><Button variant="ghost" size="icon-xs" className="editor-close" aria-label={`Закрыть ${d.path}`} title="Закрыть" onClick={()=>textOf(d)!==d.saved?setClosing(d):void close(d)}><X/></Button></div>)}</div><div className="editor-tabs-end"><UiSlot slot="editor.tabs.end" scope={workspaceScope} componentProps={{sessionId,activePath:doc?.path??null}}/></div></div>{(doc?.error||s.error)&&<div className="editor-error" role="alert">{doc?.error||s.error}</div>}<div hidden={!doc?.readOnlyReason} className="editor-hint" role="status">{doc?.readOnlyReason} — только чтение</div><Sheet open={!!closing} onOpenChange={open=>{if(!open)setClosing(null);}}><SheetContent side="right"><SheetHeader><SheetTitle>Закрыть файл?</SheetTitle><SheetDescription>В файле {closing?.path} есть несохранённые изменения.</SheetDescription></SheetHeader><SheetFooter><Button variant="outline" onClick={()=>setClosing(null)}>Отмена</Button><Button variant="destructive" onClick={()=>{if(closing)void close(closing,true);setClosing(null);}}>Закрыть без сохранения</Button></SheetFooter></SheetContent></Sheet><div className="editor-canvas" ref={element} hidden={!doc||!!doc.preview}/>{doc?.preview&&<Preview key={doc.path} doc={doc}/>}{!doc&&<div className="editor-empty"><CodePlaceholder/><span>Выберите файл в дереве проекта</span></div>}</div>;
}
function Preview({doc}:{doc:Doc}){
 const [size,setSize]=useState('');const name=doc.path.split('/').pop();const {url,mime,size:bytes}=doc.preview!;
 const caption=<figcaption>{name}{size&&` · ${size}`} · {Math.max(1,Math.round(bytes/1024))} КБ</figcaption>;
 if(mime==='application/pdf')return <figure className="editor-image"><iframe className="editor-pdf" src={url} title={name}/>{caption}</figure>;
 return <figure className="editor-image"><div className="editor-image-stage"><img src={url} alt={name} onLoad={event=>setSize(`${event.currentTarget.naturalWidth}×${event.currentTarget.naturalHeight}`)}/></div>{caption}</figure>;
}
function WorkspaceTrees({sessionId}:{sessionId:string}){
 const workspaces=useSyncExternalStore(models.workspaces.subscribe,models.workspaces.getSnapshot,models.workspaces.getSnapshot);
 const current=workspaces.find(w=>w.sessionIds.includes(sessionId))?.id;
 // Untouched groups follow the active workspace: it is open, the others are folded.
 const [toggled,setToggled]=useState<Record<string,boolean>>({});
 useEffect(()=>{void models.workspaces.refresh().catch(()=>{});},[]);
 return <ScrollArea className="editor-tree"><div className="editor-tree-title">Файлы</div>{!workspaces.some(w=>w.path)&&<p className="editor-hint">Создайте рабочее пространство с папкой проекта, чтобы увидеть файлы.</p>}{workspaces.filter(w=>w.path).map(workspace=>{
  const open=toggled[workspace.id]??workspace.id===current;
  // The backend resolves a workspace through one of its sessions; prefer the active one.
  const owner=workspace.id===current?sessionId:workspace.sessionIds.find(id=>!workspace.archivedSessionIds.includes(id))??workspace.sessionIds[0];
  return <section className="editor-workspace" key={workspace.id} aria-label={workspace.title}>
   <button className="editor-file editor-workspace-toggle" aria-expanded={open} title={workspace.path??workspace.title} onClick={()=>setToggled(value=>({...value,[workspace.id]:!open}))}><ChevronRight size={13} className={open?'expanded':''}/>{open?<FolderOpen size={15}/>:<Folder size={15}/>}<span>{workspace.title}</span></button>
   {open&&(workspace.missing?<p className="editor-hint">Папка недоступна</p>:!owner?<p className="editor-hint">Создайте диалог в этом пространстве, чтобы открыть файлы</p>:<Tree view={sessionId} sessionId={owner} depth={1}/>)}
  </section>;
 })}</ScrollArea>;
}
function CodePlaceholder(){return <span className="editor-empty-glyph">{'</>'}</span>;}
export default function EditorView({sessionId,placement}:{sessionId:string;placement:'sidebar'|'body'}){
 return placement==='sidebar'?<WorkspaceTrees sessionId={sessionId}/>:<Editor sessionId={sessionId}/>;
}
