import {viteWorkerStub} from './vite-worker-stub.mjs';
import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import {readFile} from 'node:fs/promises';

async function mount(page,contents,request){
 await page.exposeFunction('request',request);
 const bundle=await build({plugins:[viteWorkerStub],stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`import React from 'react';import {createRoot} from 'react-dom/client';import {configureClient} from './apps/web/src/client';configureClient({call:(method,params)=>window.request(method,params)});${contents}`},bundle:true,write:false,format:'esm',loader:{'.css':'empty'}});
 await page.route('http://harness.test/**',route=>route.fulfill({contentType:route.request().url().endsWith('app.js')?'text/javascript':'text/html',body:route.request().url().endsWith('app.js')?bundle.outputFiles[0].text:'<div id="root"></div><script type="module" src="/app.js"></script>'}));
 await page.goto('http://harness.test/');
}
test('workspace menu exposes its target to plugins and removes contributions on disable',async({page})=>{
 await mount(page,`
 import {WorkspaceMenu} from './plugins/mh-workspaces/ui/WorkspaceMenu';
 import {DropdownMenuItem} from './apps/web/src/components/ui/dropdown-menu';
 import {PluginRoot,workspaceScope,createBrowserComposition} from './apps/web/src/app/plugins';
 const host=await createBrowserComposition();
 await host.activate([{id:'workspace-test',apply(ctx){ctx.scope(workspaceScope).contextMenus.register({id:'workspace-test.action',target:'workspace',order:200,resolve:({workspace})=>[{id:'extra',label:'Plugin workspace action',run:()=>{window.picked=workspace.id;}}]});}}]);
 window.disableMenuPlugin=()=>host.disable('workspace-test');
 createRoot(document.getElementById('root')).render(<PluginRoot host={host}><WorkspaceMenu workspace={{id:'target',title:'Target',path:null,missing:false,sessionIds:[]}} onRemoved={()=>{}}><button>Target</button></WorkspaceMenu></PluginRoot>);`,async()=>[]);
 await page.getByRole('button',{name:'Target',exact:true}).click({button:'right'});
 await expect(page.getByRole('menuitem')).toHaveText(['Переименовать','Plugin workspace action','Локальные навыки','Удалить workspace']);
 await page.getByRole('menuitem',{name:'Plugin workspace action'}).click();
 expect(await page.evaluate(()=>window.picked)).toBe('target');await expect(page.getByRole('menu')).toHaveCount(0);
 await page.evaluate(()=>window.disableMenuPlugin());
 const trigger=page.getByRole('button',{name:'Меню workspace — Target'});
 await trigger.focus();await page.keyboard.press('Enter');
 await expect(page.getByRole('menuitem',{name:'Plugin workspace action'})).toHaveCount(0);
 await expect(page.getByRole('menuitem',{name:'Локальные навыки',exact:true})).toBeVisible();
 await page.keyboard.press('Escape');await expect(trigger).toBeFocused();
 const row=page.getByRole('button',{name:'Target',exact:true});await row.focus();await page.keyboard.press('Shift+F10');
 await expect(page.getByRole('menuitem',{name:'Переименовать',exact:true})).toBeVisible();
 await page.keyboard.press('Escape');await expect(row).toBeFocused();
});

test('Ollama preset discovers and saves models without an API key',async({page})=>{
 const calls=[];
 await mount(page,`
 import {Settings} from './apps/web/src/pages/settings/Settings';
 import {PluginRoot,createBrowserComposition} from './apps/web/src/app/plugins';
 const host=await createBrowserComposition();
 function App(){const [providers,setProviders]=React.useState([]);return <PluginRoot host={host}><Settings placement="body" tab="models" onSelectPage={()=>{}} providers={providers} onChange={setProviders} activePlugins={[]}/></PluginRoot>;}
 createRoot(document.getElementById('root')).render(<App/>);`,async(method,params)=>{
  calls.push([method,params]);
  if(method==='discover_models')return ['qwen3:8b'];
  if(method==='save_model_provider')return [{...params.provider,hasKey:false}];
  throw Error(method);
 });
 await page.getByRole('button',{name:'Добавить провайдера моделей'}).click();
 await page.getByRole('combobox',{name:'Шаблон',exact:true}).click();
 await page.getByRole('option',{name:'Ollama',exact:true}).click();
 await expect(page.getByLabel('Базовый URL')).toHaveValue('http://localhost:11434/v1');
 await expect(page.getByLabel('API-ключ (необязательно)')).toHaveValue('');
 await page.getByRole('button',{name:'Получить доступные модели'}).click();
 await page.getByRole('checkbox',{name:'qwen3:8b'}).check();
 await page.getByRole('button',{name:'Сохранить',exact:true}).click();
 await expect(page.getByText('qwen3:8b · Без API-ключа')).toBeVisible();
 expect(calls.find(([method])=>method==='save_model_provider')[1].provider).toMatchObject({id:'ollama',protocol:'ollama',apiKey:'',models:['qwen3:8b']});
});

test('composer menu supports keyboard, plugin items, dismissal and disabled state',async({page})=>{
 await mount(page,`
 import {AssistantRuntimeProvider,useLocalRuntime,ComposerPrimitive} from '@assistant-ui/react';
 import {AttachmentPicker} from './apps/web/src/harness/AttachmentPicker';
 import {ComposerMenuItem} from './apps/web/src/components/assistant-ui/composer-menu';
 import {PluginRoot,UiSlot,workspaceScope,createBrowserComposition} from './apps/web/src/app/plugins';
 const host=await createBrowserComposition();
 await host.activate([{id:'menu-test',apply(ctx){ctx.scope(workspaceScope).slots.register('composer.menu',{entryId:'extra',component:({onClose})=><ComposerMenuItem onClick={()=>{window.picked=true;onClose();}}>Plugin action</ComposerMenuItem>});}}]);
 window.disablePlugin=()=>host.disable('menu-test');
 function App(){const runtime=useLocalRuntime({async *run(){}});const [disabled,setDisabled]=React.useState(false);window.lock=()=>setDisabled(true);return <PluginRoot host={host}><AssistantRuntimeProvider runtime={runtime}><ComposerPrimitive.Root><ComposerPrimitive.Input/><AttachmentPicker disabled={disabled} renderMenu={onClose=><UiSlot slot="composer.menu" scope={workspaceScope} componentProps={{sessionId:'s',disabled,onClose}}/>}/></ComposerPrimitive.Root></AssistantRuntimeProvider></PluginRoot>;}
 createRoot(document.getElementById('root')).render(<App/>);`,async()=>[]);
 const trigger=page.getByRole('button',{name:'Добавить вложение'});
 await trigger.focus();await page.keyboard.press('Enter');
 await expect(page.getByRole('menuitem',{name:'Прикрепить файл'})).toBeFocused();
 await page.keyboard.press('ArrowDown');await expect(page.getByRole('menuitem',{name:'Plugin action'})).toBeFocused();
 await page.keyboard.press('Enter');await expect(page.getByRole('menu')).toHaveCount(0);expect(await page.evaluate(()=>window.picked)).toBe(true);
 await trigger.click();await page.keyboard.press('Escape');await expect(trigger).toBeFocused();
 await page.getByRole('textbox').fill('Keep my draft');await trigger.click();await page.getByRole('textbox').click();
 await expect(page.getByRole('menu')).toHaveCount(0);await expect(page.getByRole('textbox')).toHaveValue('Keep my draft');await expect(page.getByRole('textbox')).toBeFocused();
 await page.evaluate(()=>window.disablePlugin());await trigger.click();await expect(page.getByRole('menuitem',{name:'Plugin action'})).toHaveCount(0);
 await page.keyboard.press('Escape');await page.evaluate(()=>window.lock());await expect(trigger).toBeDisabled();
});
test('session controls preserve workspace and save execution settings and expose available tools',async({page})=>{
 const calls=[];
 await mount(page,`import {SessionPanel} from './apps/web/src/harness/SessionPanel';createRoot(document.getElementById('root')).render(<SessionPanel sessionId="session-a"/>);`,async(method,params)=>{calls.push([method,params]);if(method==='workspace.get')return {cwd:'D:\\fixture',permissionPolicy:'read-only',contextWindow:32768,autoCompact:true};if(method==='workspace.save')return params.settings;if(method==='tools.list')return [{name:'read_file',description:'Read a file',sideEffect:false}];throw Error(method);});
 await expect(page.getByLabel('Рабочая папка',{exact:true})).toHaveCount(0);
 await expect(page.getByRole('combobox',{name:'Политика выполнения'})).toHaveCount(0);
 await page.getByLabel('Окно контекста (токены)').fill('16384');const autoCompact=page.getByRole('switch',{name:'Автоматически сжимать контекст'});await autoCompact.click();await expect(autoCompact).toHaveAttribute('aria-checked','false');
 await page.getByRole('button',{name:'Сохранить настройки сессии'}).click();await expect(page.getByRole('status')).toHaveText('Настройки сохранены.');
 expect(calls.find(([m])=>m==='workspace.save')[1]).toEqual({sessionId:'session-a',settings:{cwd:'D:\\fixture',permissionPolicy:'read-only',contextWindow:16384,autoCompact:false}});
 await expect(page.getByText('Доступные инструменты · 1')).toBeVisible();await expect(page.getByText('Read a file',{exact:false})).toBeVisible();
});
test('interaction UI is contributed by a disposable browser plugin',async({page})=>{
 await mount(page,`import {createBrowserComposition,PluginRoot,UiSlot,workspaceScope} from './apps/web/src/app/plugins';const host=await createBrowserComposition();window.disableInteractions=()=>host.disable('interactions-ui');createRoot(document.getElementById('root')).render(<PluginRoot host={host}><UiSlot slot="composer.access" scope={workspaceScope} componentProps={{sessionId:'session-a',disabled:false}}/><UiSlot slot="session.controls" scope={workspaceScope} componentProps={{sessionId:'session-a',running:false}}/></PluginRoot>);`,async(method)=>{if(method==='workspace.get')return {cwd:null,permissionPolicy:'ask',contextWindow:32768,autoCompact:true};if(method==='interactions.list')return [];if(method==='context.status')return {estimatedTokens:0,contextWindow:32768,availableTokens:32768,usage:{inputTokens:1,outputTokens:1,totalTokens:2}};return [];});
 await expect(page.getByRole('combobox',{name:'Доступ сессии'})).toBeVisible();await expect(page.getByRole('region',{name:'Управление сессией'})).toBeVisible();
 await page.evaluate(()=>window.disableInteractions());await expect(page.getByRole('combobox',{name:'Доступ сессии'})).toHaveCount(0);await expect(page.getByRole('region',{name:'Управление сессией'})).toHaveCount(0);
});

test('composer access saves only its session and preserves fresh context settings',async({page})=>{
 const calls=[];let settings={cwd:'D:\\fixture',permissionPolicy:'read-only',contextWindow:16384,autoCompact:false};
 await mount(page,`import {SessionAccess} from './plugins/mh-interactions/ui/SessionAccess';createRoot(document.getElementById('root')).render(<SessionAccess sessionId="session-a"/>);`,async(method,params)=>{calls.push([method,params]);if(method==='workspace.get')return settings;if(method==='workspace.save'){settings=params.settings;return settings;}throw Error(method);});
 await page.getByRole('combobox',{name:'Доступ сессии'}).click();await page.getByRole('option',{name:'Полный доступ',exact:true}).click();
 await expect(page.getByRole('combobox')).toContainText('Полный доступ');
 expect(calls.find(([m])=>m==='workspace.save')[1]).toEqual({sessionId:'session-a',settings:{cwd:'D:\\fixture',permissionPolicy:'allow',contextWindow:16384,autoCompact:false}});
});

test('assistant-ui tool approval answers exact run and call and retries failed answers',async({page})=>{
 const calls=[];let failed=false,answered=false;
 const approval={id:'approval',sessionId:'session-a',runId:'run-7',callId:'call-9',kind:'approval',title:'write',prompt:'Change fixture.txt',options:[],createdAt:0};
 await mount(page,`
 import {AssistantRuntimeProvider,useLocalRuntime,ThreadPrimitive,MessagePrimitive} from '@assistant-ui/react';
 import {ToolAccess} from './plugins/mh-interactions/ui/ToolAccess'; import {decodeSessionMessage} from './apps/web/src/sessionHistory';
 function Fixture(){const runtime=useLocalRuntime({async run(){throw Error('must not regenerate');}});React.useEffect(()=>{runtime.thread.import({messages:[decodeSessionMessage({id:'run-7',parentId:null,role:'assistant',createdAt:new Date().toISOString(),parts:[{kind:'tool-call',data:{callId:'call-9',name:'write',arguments:{},status:'requires-approval'}}],extensions:{execution:{status:'running'}}})]});},[runtime]);return <AssistantRuntimeProvider runtime={runtime}><ThreadPrimitive.Root><ThreadPrimitive.Messages>{()=> <MessagePrimitive.Root><MessagePrimitive.Parts components={{tools:{Fallback:()=> <ToolAccess sessionId="session-a" callId="call-9"/>}}}/></MessagePrimitive.Root>}</ThreadPrimitive.Messages></ThreadPrimitive.Root></AssistantRuntimeProvider>;}createRoot(document.getElementById('root')).render(<Fixture/>);
 `,async(method,params)=>{calls.push([method,params]);if(method==='interactions.list')return answered?[]:[{...approval,id:'other',runId:'other-run'},approval];if(method==='interactions.answer'){if(!failed){failed=true;throw Error('offline');}answered=true;return true;}throw Error(method);});
 await expect(page.getByRole('region',{name:'Разрешение на выполнение'})).toBeVisible();
 await page.getByRole('button',{name:'Разрешить этот вызов'}).click();await expect(page.getByRole('alert')).toBeVisible();
 await page.getByRole('button',{name:'Разрешить этот вызов'}).click();await expect(page.getByRole('region',{name:'Разрешение на выполнение'})).toHaveCount(0);
 expect(calls.filter(([m])=>m==='interactions.answer').map(([,p])=>p)).toEqual(Array(2).fill({sessionId:'session-a',id:'approval',decision:'allow',answer:null}));
});

test('pending approvals and questions answer exact identities and survive a failed poll',async({page})=>{
 const calls=[];let failed=false,answered=new Set();
 const approval={id:'approval',sessionId:'session-a',runId:'run-7',callId:'call-9',kind:'approval',title:'Write file?',prompt:'Change fixture.txt',options:[],createdAt:0};
 const question={...approval,id:'question',kind:'question',title:'Which branch?',prompt:'Choose the target',options:['main','release']};
 await mount(page,`import {SessionControls} from './plugins/mh-interactions/ui/SessionControls';createRoot(document.getElementById('root')).render(<SessionControls sessionId="session-a"/>);`,async(method,params)=>{calls.push([method,params]);if(method==='context.status')return {contextWindow:32768,estimatedTokens:123,availableTokens:32645,shouldCompact:false,usage:{inputTokens:77,outputTokens:12,totalTokens:89}};if(method==='interactions.list'){if(!failed){failed=true;throw Error('offline');}return [approval,question].filter(item=>!answered.has(item.id));}if(method==='interactions.answer'){answered.add(params.id);return true;}throw Error(method);});
 await expect(page.getByText(/Переподключение к сессии/)).toBeVisible();
 await expect(page.getByText('Write file?',{exact:true})).toBeVisible();
 await expect(page.getByText(/77.*12.*89/)).toBeVisible();
 await page.getByRole('button',{name:'Разрешить этот вызов'}).click();await page.getByRole('radio',{name:'release'}).check();await page.getByRole('button',{name:'Отправить ответ'}).click();
 expect(calls.filter(([m])=>m==='interactions.answer').map(([,p])=>p)).toEqual([{sessionId:'session-a',id:'approval',decision:'allow',answer:null},{sessionId:'session-a',id:'question',decision:'answer',answer:'release'}]);
 await expect(page.getByText('Write file?',{exact:true})).toHaveCount(0);await expect(page.getByText('Which branch?',{exact:true})).toHaveCount(0);
});
test('MCP transport editing sends secrets once and never restores them into forms',async({page})=>{
 const calls=[];let servers=[];
 await mount(page,`import {McpPanel} from './apps/web/src/harness/McpPanel';createRoot(document.getElementById('root')).render(<McpPanel/>);`,async(method,params)=>{calls.push([method,params]);if(method==='mcp.list')return servers;if(method==='mcp.save'){const {credentials,...config}=params.server;servers=[{...config,state:'disabled',toolCount:0,hasCredentials:!!credentials}];return servers;}throw Error(method);});
 await page.getByRole('button',{name:'Добавить сервер MCP'}).click();await page.getByLabel('ID сервера').fill('fixture');await page.getByLabel('Команда',{exact:true}).fill('node');
 await page.getByRole('combobox',{name:'Учётные данные MCP'}).click();await page.getByRole('option',{name:'Заменить учётные данные'}).click();await page.getByLabel('Секретные значения MCP').fill('TOKEN=one-time-secret');await page.getByRole('button',{name:'Сохранить сервер MCP'}).click();
 await expect(page.getByText('Учётные данные сохранены',{exact:false})).toBeVisible();await page.getByRole('button',{name:'Изменить',exact:true}).click();
 await expect(page.getByLabel('Секретные значения MCP')).toHaveCount(0);await expect(page.locator('body')).not.toContainText('one-time-secret');
 await page.getByRole('button',{name:'Сохранить сервер MCP'}).click();
 const saves=calls.filter(([m])=>m==='mcp.save');expect(saves[0][1].server.credentials.env).toEqual({TOKEN:'one-time-secret'});expect(saves[1][1].server.credentials).toBeNull();
});
test('actual composer sends an image-only turn and history hydrates raster refs lazily',async({page})=>{
 const calls=[];const pixel='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=';
 const ref={id:'image-id',name:'pixel.png',mimeType:'image/png',bytes:68};
 await mount(page,`
 import {AssistantRuntimeProvider,useLocalRuntime,ThreadPrimitive,MessagePrimitive,ComposerPrimitive,AttachmentPrimitive} from '@assistant-ui/react';
 import {AttachmentPicker} from './apps/web/src/harness/AttachmentPicker';import {attachmentAdapter} from './apps/web/src/harness/attachments';import {SessionImage} from './apps/web/src/harness/SessionImage';import {chatAdapter} from './apps/web/src/chatAdapter';import {sessionHistory,decodeSessionMessage} from './apps/web/src/sessionHistory';import {invoke} from './apps/web/src/client';
 function App(){const history=React.useMemo(()=>sessionHistory('session-a',()=>{},()=>{},invoke,{serverOwned:true}),[]);const adapter=React.useMemo(()=>chatAdapter({invoke,sessionId:'session-a',model:'api:p/m',options:{},history,onRunning:()=>{},onSaved:()=>{},onError:e=>{window.fixtureError=e;},serverOwned:true}),[history]);const attachments=React.useMemo(()=>attachmentAdapter('session-a',invoke),[]);const runtime=useLocalRuntime(adapter,{adapters:{history,attachments}});
 React.useEffect(()=>{window.fixtureRuntime=runtime;window.loadSaved=()=>runtime.thread.import({messages:[decodeSessionMessage({id:'saved',parentId:null,role:'user',createdAt:new Date().toISOString(),parts:[{kind:'image',data:${JSON.stringify(ref)}}],extensions:{}})]});},[runtime]);
 return <AssistantRuntimeProvider runtime={runtime}><ThreadPrimitive.Root><ThreadPrimitive.Messages>{()=> <MessagePrimitive.Root><MessagePrimitive.Parts components={{Image:props=><SessionImage {...props} sessionId="session-a"/>}}/></MessagePrimitive.Root>}</ThreadPrimitive.Messages><ComposerPrimitive.Root><ComposerPrimitive.Attachments>{()=> <AttachmentPrimitive.Root><AttachmentPrimitive.Name/><AttachmentPrimitive.unstable_Thumb/><AttachmentPrimitive.Remove>Удалить</AttachmentPrimitive.Remove></AttachmentPrimitive.Root>}</ComposerPrimitive.Attachments><ComposerPrimitive.Input/><AttachmentPicker/><ComposerPrimitive.Send>Send</ComposerPrimitive.Send></ComposerPrimitive.Root></ThreadPrimitive.Root></AssistantRuntimeProvider>;}
 createRoot(document.getElementById('root')).render(<App/>);`,async(method,params)=>{calls.push([method,params]);if(method==='session_load')return [];if(method==='attachments.put')return ref;if(method==='attachments.get')return {...ref,data:pixel};if(method==='runs.start')return {sessionId:'session-a',runId:params.requestId};if(method==='runs.get')return {...params,status:'completed',parts:[{kind:'text',data:{text:'Image received'}}],error:null};throw Error(method);});
 await page.waitForFunction(()=>window.fixtureRuntime&&!window.fixtureRuntime.thread.getState().isLoading);
 const chooser=page.waitForEvent('filechooser');await page.getByRole('button',{name:'Добавить вложение'}).click();await page.getByRole('menuitem',{name:'Прикрепить файл'}).click();await (await chooser).setFiles({name:'pixel.png',mimeType:'image/png',buffer:Buffer.from(pixel,'base64')});
 await expect(page.getByRole('button',{name:'Удалить',exact:true})).toBeVisible();await page.getByRole('button',{name:'Send',exact:true}).click();await expect(page.getByText('Image received')).toBeVisible();
 const request=calls.find(([method])=>method==='runs.start')[1];expect(request.options.attachments).toEqual([ref]);expect(request.message.text).toBe('');expect(JSON.stringify(request)).not.toContain(pixel);
 await page.evaluate(()=>window.loadSaved());await expect(page.locator('img[alt="pixel.png"]')).toBeVisible();expect(calls.filter(([method])=>method==='attachments.get')).toHaveLength(1);
});
test('generated image appears in assistant-ui and reloads from its session attachment',async({page},info)=>{
 const calls=[];const pixel='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=';
 await page.addInitScript(()=>{window.ClipboardItem=class {constructor(items){this.types=Object.keys(items);}};Object.defineProperty(navigator,'clipboard',{value:{write:async items=>{window.copiedImageTypes=items[0].types;}}});});
 const attachment={id:'generated-id',name:'generated.png',mimeType:'image/png',bytes:68};
 const saved={id:'run-generated',parentId:null,role:'assistant',createdAt:'2026-11-10T10:00:00Z',parts:[{kind:'tool-call',data:{callId:'image-call',name:'generate_image',arguments:{prompt:'fixture'},status:'completed'}},{kind:'tool-result',data:{callId:'image-call',output:{status:'completed',attachment},isError:false}}],extensions:{execution:{status:'completed'}}};
 await mount(page,`
 import {AssistantRuntimeProvider,useLocalRuntime,ThreadPrimitive,MessagePrimitive} from '@assistant-ui/react';
 import {SessionImage} from './apps/web/src/harness/SessionImage';import {decodeSessionMessage} from './apps/web/src/sessionHistory';
 function App(){const runtime=useLocalRuntime({async run(){throw Error('generation must not restart');}});React.useEffect(()=>{runtime.thread.import({messages:[decodeSessionMessage(${JSON.stringify(saved)})]});},[runtime]);return <AssistantRuntimeProvider runtime={runtime}><ThreadPrimitive.Root><ThreadPrimitive.Messages>{()=> <MessagePrimitive.Root><MessagePrimitive.Parts components={{Image:props=><SessionImage {...props} sessionId="session-a"/>,tools:{Fallback:()=>null}}}/></MessagePrimitive.Root>}</ThreadPrimitive.Messages></ThreadPrimitive.Root></AssistantRuntimeProvider>;}createRoot(document.getElementById('root')).render(<App/>);
 `,async(method,params)=>{calls.push([method,params]);if(method==='attachments.get'&&params.sessionId==='session-a'&&params.id===attachment.id)return {...attachment,data:pixel};throw Error(method);});
 await page.addStyleTag({content:await readFile('apps/web/src/execution/execution.css','utf8')});
 await expect(page.getByRole('img',{name:'generated.png'})).toBeVisible();
 await expect(page.getByRole('link',{name:'Скачать'})).toHaveAttribute('download','generated.png');
 await page.getByRole('button',{name:'Увеличить изображение'}).click();await expect(page.getByRole('dialog',{name:'Изображение'})).toBeVisible();await expect(page.getByRole('dialog',{name:'Изображение'}).getByRole('button')).toHaveCount(1);
 expect(await page.getByRole('dialog',{name:'Изображение'}).evaluate(el=>getComputedStyle(el,'::backdrop').backdropFilter)).toBe('blur(18px)');
 await page.screenshot({path:info.outputPath('image-modal.png')});await page.getByRole('button',{name:'Закрыть просмотр'}).click();
 await page.getByRole('button',{name:'Копировать'}).click();await expect.poll(()=>page.evaluate(()=>window.copiedImageTypes)).toEqual(['image/png']);
 await page.screenshot({path:info.outputPath('generated.png')});await page.reload();
 await expect(page.getByRole('img',{name:'generated.png'})).toBeVisible();
 expect(calls.filter(([method])=>method==='attachments.get')).toHaveLength(2);
});
test('image generation shows an animated square and respects reduced motion',async({page},info)=>{
 await mount(page,`import {ImageGenerating} from './apps/web/src/components/assistant-ui/elements/image';createRoot(document.getElementById('root')).render(<ImageGenerating callId="generation"/>);`,async()=>[]);
 await page.addStyleTag({content:'body{margin:0;background:#191919;padding:48px}'+await readFile('apps/web/src/execution/execution.css','utf8')});
 await page.setViewportSize({width:900,height:750});const loading=page.getByRole('status',{name:'Создание изображения'});
 await expect(loading).toBeVisible();const rect=await loading.boundingBox();expect(rect.width).toBeCloseTo(rect.height,0);
 expect(await loading.evaluate(el=>getComputedStyle(el,'::before').animationName)).toBe('image-glow');
 const before=await loading.evaluate(el=>getComputedStyle(el,'::before').transform);await page.waitForTimeout(600);
 expect(await loading.evaluate(el=>getComputedStyle(el,'::before').transform)).not.toBe(before);
 await page.screenshot({path:info.outputPath('generation-desktop.png')});await page.emulateMedia({reducedMotion:'reduce'});
 expect(await loading.evaluate(el=>getComputedStyle(el,'::before').animationName)).toBe('none');
 await page.setViewportSize({width:390,height:844});const mobile=await loading.boundingBox();expect(mobile.width).toBeCloseTo(mobile.height,0);expect(mobile.x+mobile.width).toBeLessThanOrEqual(390);
 await page.screenshot({path:info.outputPath('generation-mobile.png')});
});
test('image viewer shows only the image and a close icon over a blurred backdrop',async({page},info)=>{
 const source=(await readFile('apps/web/public/minerva-welcome.png')).toString('base64');
 await mount(page,`import {Image} from './apps/web/src/components/assistant-ui/elements/image';createRoot(document.getElementById('root')).render(<div style={{padding:50,background:'#252229',minHeight:'100vh'}}><p>Чат на заднем плане</p><Image type="image" image="data:image/png;base64,${source}" filename="example.png"/></div>);`,async()=>[]);
 await page.addStyleTag({content:await readFile('apps/web/src/execution/execution.css','utf8')});
 await page.setViewportSize({width:900,height:700});await page.getByRole('button',{name:'Увеличить изображение'}).click();
 const dialog=page.getByRole('dialog',{name:'Изображение'});await expect(dialog.getByRole('img',{name:'example.png'})).toBeVisible();
 await expect(dialog.getByRole('button')).toHaveCount(1);await expect(dialog.getByRole('button',{name:'Закрыть просмотр'})).toBeVisible();
 expect(await dialog.evaluate(el=>getComputedStyle(el,'::backdrop').backdropFilter)).toBe('blur(18px)');
 const desktop=await dialog.getByRole('img',{name:'example.png'}).boundingBox();expect(desktop.x).toBeGreaterThanOrEqual(0);expect(desktop.y).toBeGreaterThanOrEqual(0);expect(desktop.x+desktop.width).toBeLessThanOrEqual(900);expect(desktop.y+desktop.height).toBeLessThanOrEqual(700);
 await page.screenshot({path:info.outputPath('viewer-desktop.png')});await page.setViewportSize({width:390,height:844});
 const image=await dialog.getByRole('img',{name:'example.png'}).boundingBox();expect(image.x).toBeGreaterThanOrEqual(0);expect(image.y).toBeGreaterThanOrEqual(0);expect(image.x+image.width).toBeLessThanOrEqual(390);expect(image.y+image.height).toBeLessThanOrEqual(844);
 await page.screenshot({path:info.outputPath('viewer-mobile.png')});await page.keyboard.press('Escape');await expect(dialog).not.toBeVisible();
});
test('copy converts a JPEG image to clipboard-compatible PNG',async({page})=>{
 await page.addInitScript(()=>{window.ClipboardItem=class {constructor(items){this.types=Object.keys(items);this.png=items['image/png'];}};Object.defineProperty(navigator,'clipboard',{value:{write:async items=>{window.copiedImage={types:items[0].types,mime:items[0].png.type,size:items[0].png.size};}}});});
 await mount(page,`import {Image} from './apps/web/src/components/assistant-ui/elements/image';const canvas=document.createElement('canvas');canvas.width=2;canvas.height=2;canvas.getContext('2d').fillRect(0,0,2,2);createRoot(document.getElementById('root')).render(<Image type="image" image={canvas.toDataURL('image/jpeg')} filename="photo.jpg"/>);`,async()=>[]);
 await page.getByRole('button',{name:'Копировать'}).click();await expect.poll(()=>page.evaluate(()=>window.copiedImage)).toMatchObject({types:['image/png'],mime:'image/png'});
 expect((await page.evaluate(()=>window.copiedImage)).size).toBeGreaterThan(0);
});
test('offscreen image refs stay lazy and late responses cannot cross session scopes',async({page})=>{
 const calls=[];let resolveFirst,resolveSecond;
 const pixel='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=';
 await mount(page,`import {SessionImage} from './apps/web/src/harness/SessionImage';function App(){const [sessionId,setSession]=React.useState('first');React.useEffect(()=>{window.switchImageSession=()=>setSession('second');},[]);return <><div style={{height:2000}}>Previous messages</div><SessionImage sessionId={sessionId} image="" type="image" providerMetadata={{minerva:{attachment:{id:'image',name:sessionId+'.png',mimeType:'image/png',bytes:68}}}}/></>;}createRoot(document.getElementById('root')).render(<App/>);`,async(method,params)=>{calls.push([method,params]);return await new Promise(resolve=>{if(params.sessionId==='first')resolveFirst=resolve;else resolveSecond=resolve;});});
 expect(calls).toHaveLength(0);await page.evaluate(()=>window.scrollTo(0,document.body.scrollHeight));await expect.poll(()=>calls.length).toBe(1);
 await page.evaluate(()=>window.switchImageSession());await expect.poll(()=>calls.length).toBe(2);
 resolveFirst({id:'image',name:'first.png',mimeType:'image/png',bytes:68,data:pixel});await expect(page.locator('img')).toHaveCount(0);
 resolveSecond({id:'image',name:'second.png',mimeType:'image/png',bytes:68,data:pixel});await expect(page.locator('img[alt="second.png"]')).toBeVisible();
 expect(calls.map(([,params])=>params.sessionId)).toEqual(['first','second']);
});
