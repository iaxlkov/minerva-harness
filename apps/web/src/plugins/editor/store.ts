/// <reference types="vite/client" />
import * as monaco from 'monaco-editor';
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';
import JsonWorker from 'monaco-editor/language/json/json.worker.js?worker';
import CssWorker from 'monaco-editor/language/css/css.worker.js?worker';
import HtmlWorker from 'monaco-editor/language/html/html.worker.js?worker';
import TsWorker from 'monaco-editor/language/typescript/ts.worker.js?worker';
import {invoke} from '../../client';
(globalThis as typeof globalThis & {MonacoEnvironment?:unknown}).MonacoEnvironment={getWorker:(_id:string,label:string)=>label==='json'?new JsonWorker():['css','scss','less'].includes(label)?new CssWorker():['html','handlebars','razor'].includes(label)?new HtmlWorker():['typescript','javascript'].includes(label)?new TsWorker():new EditorWorker()};
// sessionId owns the file (its workspace answers read/save/LSP); view is the editor whose tabs show it.
export type Doc={key:string;view:string;path:string;sessionId:string;model:monaco.editor.ITextModel;diskVersion:string;saved:string;lspVersion:number;lsp:boolean;pending:Promise<void>;error:string;preview?:{url:string;mime:string;size:number};readOnlyReason?:string;transient:boolean};
export type State={docs:Doc[];active:string;error:string;loading:string;pin:string;reveal?:{key:string;line:number;column:number}};
export function textOf(doc:Doc){return doc.model.getValue(monaco.editor.EndOfLinePreference.TextDefined,true);}
const states=new Map<string,State>();const listeners=new Set<()=>void>();let tick=0;
window.addEventListener('beforeunload',event=>{if([...states.values()].some(s=>s.docs.some(doc=>textOf(doc)!==doc.saved))){event.preventDefault();event.returnValue='';}});
export function subscribe(fn:()=>void){listeners.add(fn);return ()=>{listeners.delete(fn);};}
export function snapshot(){return tick;}
export function state(id:string):State{let s=states.get(id);if(!s){s={docs:[],active:'',error:'',loading:'',pin:''};states.set(id,s);}return s;}
export function changed(){tick++;for(const l of listeners)l();}
const languages:Record<string,string>={rs:'rust',ts:'typescript',tsx:'typescript',js:'javascript',jsx:'javascript',json:'json',py:'python',md:'markdown',html:'html',css:'css',c:'c',cpp:'cpp',h:'cpp',toml:'ini',yml:'yaml',yaml:'yaml',ps1:'powershell',sql:'sql'};
const docsByUri=new Map<string,Doc>();
// Blob URLs work for both <img> and the webview's native PDF viewer, unlike data: URLs.
function blobPreview(base64:string,mime:string){const bytes=Uint8Array.from(atob(base64),c=>c.charCodeAt(0));return {url:URL.createObjectURL(new Blob([bytes],{type:mime})),mime,size:bytes.length};}
/** Single click opens a transient preview tab that the next preview replaces; pin keeps the tab (VS Code style). */
export const docKey=(sessionId:string,path:string)=>`${sessionId}\n${path}`;
export async function open(view:string,path:string,pin=false,sessionId=view){
 const s=state(view);const key=docKey(sessionId,path);const existing=s.docs.find(d=>d.key===key);
 if(existing){if(pin)existing.transient=false;s.active=key;changed();return;}
 // A double click lands while the single click is still loading the file.
 if(s.loading){if(pin&&s.loading===key)s.pin=key;return;}
 s.loading=key;s.pin=pin?key:'';s.error='';changed();
 try{
  const result=await invoke('mh-editor.read',{sessionId,path});
  const uri=monaco.Uri.parse(`inmemory://minerva/${encodeURIComponent(sessionId)}/${path.split('/').map(encodeURIComponent).join('/')}`);
  const extension=/[^/]+\.([^./]+)$/.exec(path)?.[1]?.toLowerCase()??'';
  const language=!result.readOnlyReason&&Object.hasOwn(languages,extension)?languages[extension]:'plaintext';
  const model=monaco.editor.createModel(result.text,language,uri);
  // Previews (images, PDF) keep an empty model so tab, close and dirty tracking stay uniform.
  const doc:Doc={key,view,path,sessionId,model,diskVersion:result.version,saved:model.getValue(monaco.editor.EndOfLinePreference.TextDefined,true),readOnlyReason:result.readOnlyReason,lspVersion:1,lsp:false,pending:Promise.resolve(),error:'',preview:result.data?blobPreview(result.data,result.mimeType??'application/octet-stream'):undefined,transient:s.pin!==key};docsByUri.set(uri.toString(),doc);
  model.onDidChangeContent(()=>{doc.transient=false;const text=model.getValue();if(doc.lsp){doc.lspVersion++;const version=doc.lspVersion;doc.pending=doc.pending.then(async()=>{await invoke('mh-lsp.change',{sessionId,path,text,version});}).catch(e=>{doc.error=String(e);changed();});}changed();});
  const replaced=doc.transient?s.docs.find(d=>d.transient&&textOf(d)===d.saved):undefined;
  s.docs.splice(replaced?s.docs.indexOf(replaced)+1:s.docs.length,0,doc);s.active=key;changed();
  if(replaced)void close(replaced);
  if(doc.preview||doc.readOnlyReason)return;
  const initialText=model.getValue();
  doc.pending=(async()=>{try{await invoke('mh-lsp.open',{sessionId,path,version:1,text:initialText});doc.lsp=true;
    if(model.getValue()!==initialText){doc.lspVersion++;await invoke('mh-lsp.change',{sessionId,path,version:doc.lspVersion,text:model.getValue()});}
  }catch(e){if((e instanceof Error?e.message:String(e))!=='No language server for file')doc.error=`LSP: ${String(e)}`;}changed();})();await doc.pending;
 }catch(e){s.error=String(e);}finally{s.loading='';changed();}
}
/** Opens a file and asks the editor to put the cursor on `line`/`column` (search results). */
export async function openAt(view:string,path:string,sessionId:string,line:number,column:number){
 await open(view,path,false,sessionId);
 state(view).reveal={key:docKey(sessionId,path),line,column};changed();
}
export async function save(doc:Doc){
 if(doc.preview||doc.readOnlyReason)return;
 const text=textOf(doc);
 // Monaco normalizes line endings on open. Keep the original bytes until the user edits.
 if(text===doc.saved)return;
 try{const result=await invoke('mh-editor.save',{sessionId:doc.sessionId,path:doc.path,text,version:doc.diskVersion});doc.diskVersion=result.version;doc.saved=text;doc.error='';}catch(e){doc.error=String(e);}changed();
}
export async function close(doc:Doc,discard=false){
 if(textOf(doc)!==doc.saved&&!discard)return;
 const s=state(doc.view);await doc.pending;if(doc.lsp)await invoke('mh-lsp.close',{sessionId:doc.sessionId,path:doc.path}).catch(()=>{});
 docsByUri.delete(doc.model.uri.toString());doc.model.dispose();if(doc.preview)URL.revokeObjectURL(doc.preview.url);s.docs=s.docs.filter(d=>d!==doc);if(s.active===doc.key)s.active=s.docs.at(-1)?.key??'';changed();
}
async function query(model:monaco.editor.ITextModel,pos:monaco.Position,operation:'hover'|'completion'|'definition'|'references'){
 const doc=docsByUri.get(model.uri.toString());if(!doc?.lsp)return null;await doc.pending;return await invoke('mh-lsp.query',{sessionId:doc.sessionId,path:doc.path,operation,line:pos.lineNumber-1,character:pos.column-1});
}
const completionKinds=[monaco.languages.CompletionItemKind.Text,monaco.languages.CompletionItemKind.Text,monaco.languages.CompletionItemKind.Method,monaco.languages.CompletionItemKind.Function,monaco.languages.CompletionItemKind.Constructor,monaco.languages.CompletionItemKind.Field,monaco.languages.CompletionItemKind.Variable,monaco.languages.CompletionItemKind.Class,monaco.languages.CompletionItemKind.Interface,monaco.languages.CompletionItemKind.Module,monaco.languages.CompletionItemKind.Property,monaco.languages.CompletionItemKind.Unit,monaco.languages.CompletionItemKind.Value,monaco.languages.CompletionItemKind.Enum,monaco.languages.CompletionItemKind.Keyword,monaco.languages.CompletionItemKind.Snippet,monaco.languages.CompletionItemKind.Color,monaco.languages.CompletionItemKind.File,monaco.languages.CompletionItemKind.Reference,monaco.languages.CompletionItemKind.Folder,monaco.languages.CompletionItemKind.EnumMember,monaco.languages.CompletionItemKind.Constant,monaco.languages.CompletionItemKind.Struct,monaco.languages.CompletionItemKind.Event,monaco.languages.CompletionItemKind.Operator,monaco.languages.CompletionItemKind.TypeParameter];
const registered=new Set<string>();
export function registerLanguage(language:string){if(registered.has(language))return;registered.add(language);
 monaco.languages.registerHoverProvider(language,{async provideHover(model,pos){try{const value=await query(model,pos,'hover') as any;if(!value?.contents)return null;const values=Array.isArray(value.contents)?value.contents:[value.contents];return {contents:values.map((x:any)=>({value:typeof x==='string'?x:x.value??''}))};}catch{return null;}}});
 monaco.languages.registerDefinitionProvider(language,{async provideDefinition(model,pos){
  try{const source=docsByUri.get(model.uri.toString());if(!source)return null;const result=await query(model,pos,'definition') as any;const locations=Array.isArray(result)?result:result?[result]:[];const workspaces=await invoke('workspaces.list');const root=workspaces.find(w=>w.sessionIds.includes(source.sessionId))?.path;if(!root)return null;
   const base=root.replaceAll('\\','/').replace(/^\/\/\?\//,'').replace(/^UNC\//,'//').replace(/\/$/,'');const windows=/^[A-Za-z]:\//.test(base);
   for(const item of locations){const url=new URL(item.uri??item.targetUri);if(url.protocol!=='file:')continue;let absolute=decodeURIComponent(url.pathname);if(windows&&absolute.startsWith('/'))absolute=absolute.slice(1);const prefix=base+'/';if(!(windows?absolute.toLowerCase().startsWith(prefix.toLowerCase()):absolute.startsWith(prefix)))continue;const path=absolute.slice(prefix.length);await open(source.view,path,false,source.sessionId);const target=state(source.view).docs.find(d=>d.key===docKey(source.sessionId,path));const range=item.range??item.targetSelectionRange??item.targetRange;if(target&&range)return {uri:target.model.uri,range:new monaco.Range(range.start.line+1,range.start.character+1,range.end.line+1,range.end.character+1)};
   }return null;
  }catch{return null;}
 }});
 monaco.languages.registerCompletionItemProvider(language,{async provideCompletionItems(model,pos){try{const value=await query(model,pos,'completion') as any;const items=Array.isArray(value)?value:value?.items??[];const word=model.getWordUntilPosition(pos);const range=new monaco.Range(pos.lineNumber,word.startColumn,pos.lineNumber,word.endColumn);return {suggestions:items.filter((item:any)=>!item.additionalTextEdits?.length).map((item:any)=>{const edit=item.textEdit;const span=edit?.range??edit?.replace;const replacement=span?new monaco.Range(span.start.line+1,span.start.character+1,span.end.line+1,span.end.character+1):range;return {label:item.label,kind:completionKinds[item.kind??1]??monaco.languages.CompletionItemKind.Text,insertText:edit?.newText??item.insertText??item.label,insertTextRules:item.insertTextFormat===2?monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet:undefined,range:replacement,detail:item.detail};})};}catch{return {suggestions:[]};}}});
}
export {monaco};
