import React,{useEffect,useRef,useState,useSyncExternalStore} from 'react';
import {ChevronRight,File,Folder,FolderOpen,X} from 'lucide-react';
import {Button} from '../../components/ui/button';
import {ScrollArea} from '../../components/ui/scroll-area';
import {Sheet,SheetContent,SheetHeader,SheetTitle,SheetDescription,SheetFooter} from '../../components/ui/sheet';
import type {Doc} from './store';
import {invoke} from '../../client';
import {UiSlot,workspaceScope} from '../../app/plugins';
import {open,close,save,state,subscribe,snapshot,changed,registerLanguage,monaco,textOf} from './store';
import './editor.css';
type Entry={name:string;path:string;directory:boolean};
function Tree({sessionId,path='',depth=0}:{sessionId:string;path?:string;depth?:number}){
 const [items,setItems]=useState<Entry[]|null>(null);const [error,setError]=useState('');
 useEffect(()=>{let active=true;setItems(null);setError('');void invoke('mh-editor.list',{sessionId,path}).then(v=>{if(active)setItems(v);}).catch(e=>{if(active)setError(String(e));});return()=>{active=false;};},[sessionId,path]);
 if(error)return <p className="editor-error" role="alert">{error}</p>;
 if(!items)return <p className="editor-hint">Загрузка…</p>;
 if(items.length===0)return <p className="editor-hint">Папка пуста</p>;
 return <>{items.map(item=><TreeItem key={item.path} sessionId={sessionId} item={item} depth={depth}/>)}</>;
}
function TreeItem({sessionId,item,depth}:{sessionId:string;item:Entry;depth:number}){
 const [expanded,setExpanded]=useState(false);
 return <><button className="editor-file" title={item.path} style={{paddingLeft:8+depth*14}} onClick={()=>item.directory?setExpanded(v=>!v):void open(sessionId,item.path)}>{item.directory?<ChevronRight size={13} className={expanded?'expanded':''}/>:<span className="editor-file-spacer"/>}{item.directory?(expanded?<FolderOpen size={15}/>:<Folder size={15}/>):<File size={15}/>}<span>{item.name}</span></button>{item.directory&&expanded&&<Tree sessionId={sessionId} path={item.path} depth={depth+1}/>}</>;
}
function Editor({sessionId}:{sessionId:string}){
 useSyncExternalStore(subscribe,snapshot,snapshot);const s=state(sessionId);const doc=s.docs.find(d=>d.path===s.active);const element=useRef<HTMLDivElement>(null);const editor=useRef<monaco.editor.IStandaloneCodeEditor|null>(null);const [closing,setClosing]=useState<Doc|null>(null);
 useEffect(()=>{if(!element.current)return;const instance=monaco.editor.create(element.current,{theme:'vs-dark',automaticLayout:true,minimap:{enabled:false},fontSize:13,scrollBeyondLastLine:false,renderLineHighlight:'line',tabSize:2});editor.current=instance;return()=>{editor.current=null;instance.dispose();};},[]);
 useEffect(()=>{if(!editor.current)return;editor.current.setModel(doc?.model??null);if(doc){registerLanguage(doc.model.getLanguageId());editor.current.focus();}},[doc]);
 useEffect(()=>{if(!doc?.lsp)return;let active=true;async function poll(){try{const all=await invoke('mh-lsp.diagnostics',{sessionId,path:doc!.path}) as any[];if(!active)return;const diag=all[0]?.diagnostics??[];monaco.editor.setModelMarkers(doc!.model,'mh-lsp',diag.map((d:any)=>({message:d.message,severity:d.severity===1?monaco.MarkerSeverity.Error:d.severity===2?monaco.MarkerSeverity.Warning:monaco.MarkerSeverity.Info,startLineNumber:d.range.start.line+1,startColumn:d.range.start.character+1,endLineNumber:d.range.end.line+1,endColumn:d.range.end.character+1})));}catch{} }void poll();const timer=setInterval(()=>void poll(),2000);return()=>{active=false;clearInterval(timer);};},[sessionId,doc,doc?.lsp]);
 useEffect(()=>{const key=(event:KeyboardEvent)=>{if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='s'&&doc){event.preventDefault();void save(doc);}};window.addEventListener('keydown',key);return()=>window.removeEventListener('keydown',key);},[doc]);
 return <div className="editor-workbench"><div className="editor-tabs"><UiSlot slot="editor.tabs.start" scope={workspaceScope} componentProps={{sessionId,activePath:doc?.path??null}}/><div className="editor-tabs-list" role="tablist" aria-label="Открытые файлы">{s.docs.map(d=><div className={`editor-tab ${d===doc?'active':''}`} key={d.path}><button role="tab" aria-selected={d===doc} title={d.path} onClick={()=>{s.active=d.path;changed();}}>{d.path.split('/').pop()}{textOf(d)!==d.saved?' •':''}</button><button className="editor-close" aria-label={`Закрыть ${d.path}`} onClick={()=>textOf(d)!==d.saved?setClosing(d):void close(d)}><X size={13}/></button></div>)}</div><div className="editor-tabs-end"><UiSlot slot="editor.tabs.end" scope={workspaceScope} componentProps={{sessionId,activePath:doc?.path??null}}/></div></div>{(doc?.error||s.error)&&<div className="editor-error" role="alert">{doc?.error||s.error}</div>}<Sheet open={!!closing} onOpenChange={open=>{if(!open)setClosing(null);}}><SheetContent side="right"><SheetHeader><SheetTitle>Закрыть файл?</SheetTitle><SheetDescription>В файле {closing?.path} есть несохранённые изменения.</SheetDescription></SheetHeader><SheetFooter><Button variant="outline" onClick={()=>setClosing(null)}>Отмена</Button><Button variant="destructive" onClick={()=>{if(closing)void close(closing,true);setClosing(null);}}>Закрыть без сохранения</Button></SheetFooter></SheetContent></Sheet><div className="editor-canvas" ref={element} hidden={!doc}/>{!doc&&<div className="editor-empty"><CodePlaceholder/><span>Выберите файл в дереве проекта</span></div>}</div>;
}
function CodePlaceholder(){return <span className="editor-empty-glyph">{'</>'}</span>;}
export default function EditorView({sessionId,placement}:{sessionId:string;placement:'sidebar'|'body'}){
 return placement==='sidebar'?<ScrollArea className="editor-tree"><div className="editor-tree-title">ФАЙЛЫ ПРОЕКТА</div><Tree key={sessionId} sessionId={sessionId}/></ScrollArea>:<Editor sessionId={sessionId}/>;
}
