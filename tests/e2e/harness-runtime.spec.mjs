import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,writeFile,readFile,rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {harnessModel} from '../fixtures/harness-model.mjs';

async function runtime(page,context,body,desktop=false){
 const temp=await mkdtemp(path.join(os.tmpdir(),'minerva-harness-')),model=await harnessModel(),providerId=`harness${randomUUID().replaceAll('-','')}`;
 await mkdir(path.join(temp,'.agents/skills/fixture'),{recursive:true});
 await writeFile(path.join(temp,'AGENTS.md'),'WORKSPACE-DIRECTIVE: inspect fixture.txt before editing.');
 await writeFile(path.join(temp,'.agents/skills/fixture/SKILL.md'),'---\ndescription: fixture skill\n---\nSKILL-DIRECTIVE: keep exact fixture values.');
 await writeFile(path.join(temp,'fixture.txt'),'original fixture value');
 const env={...process.env};for(const key of ['OPENAI_API_KEY','ANTHROPIC_API_KEY','DEEPSEEK_API_KEY'])delete env[key];
 const host=spawn(process.env.MINERVA_TEST_SERVER_EXE||path.resolve('target/debug',process.platform==='win32'?'minerva-server.exe':'minerva-server'),['--bind','127.0.0.1:0','--data-dir',path.join(temp,'data'),'--config-dir',path.join(temp,'config'),'--web-dist',path.resolve('dist')],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
 let rpc,configured=false;
 try{
  const launch=await new Promise((resolve,reject)=>{
   let stdout='',stderr='';const timeout=setTimeout(()=>reject(Error(`Host startup timeout: ${stderr}`)),20000);
   host.stderr.on('data',chunk=>stderr+=chunk);host.once('error',error=>{clearTimeout(timeout);reject(error);});host.once('exit',code=>{clearTimeout(timeout);reject(Error(`Host exited ${code}: ${stderr}`));});
   host.stdout.on('data',chunk=>{stdout+=chunk;const match=stdout.match(/http:\/\/127\.0\.0\.1:\d+\/#minerva-token=[a-f0-9]+/);if(match){clearTimeout(timeout);resolve(match[0]);}});
  });
  const origin=new URL(launch).origin;
  await page.goto(launch);await expect(page.getByRole('textbox',{name:'Сообщение ассистенту'})).toBeVisible();
  const rawRpc=async(method,params={})=>{const boot=await (await context.request.get(`${origin}/api/v1/bootstrap`)).json();return await (await context.request.post(`${origin}/api/v1/rpc`,{headers:{Origin:origin,'X-Minerva-CSRF':boot.csrfToken},data:{protocolVersion:1,method,params}})).json();};
  rpc=async(method,params={})=>{const result=await rawRpc(method,params);expect(result.ok,JSON.stringify(result.error)).toBe(true);return result.value;};
  await rpc('save_model_provider',{provider:{id:providerId,name:'Harness fixture',baseUrl:model.url,models:['fixture'],apiKey:'disposable-fixture-key',isNew:true,protocol:'openai-chat'}});configured=true;
  const project=await rpc('workspaces.create',{path:temp});
  const id=randomUUID();await rpc('session_create',{id,workspaceId:project.id});
  const session=await rpc('session_get',{id});
  await rpc('session_set_model',{id:session.id,model:`api:${providerId}/fixture`});
  const settings={cwd:project.path,permissionPolicy:'ask',contextWindow:32768,autoCompact:true};
  await rpc('workspace.save',{sessionId:session.id,settings});
  if(desktop){
   await page.exposeFunction('desktopInvoke',(command,params)=>{
    if(command==='harness_rpc')return rpc(params.method,params.params);
    if(command==='run_start')return rpc('runs.start',params.request);
    const methods={run_get:'runs.get',run_cancel:'runs.cancel',plugin_call:'plugins.call',plugin_models:'plugins.models',plugin_ui:'plugins.ui'};
    return rpc(methods[command]??command,params);
   });
   await page.addInitScript(()=>{window.__TAURI_INTERNALS__={invoke:(command,params)=>window.desktopInvoke(command,params)};});
  }
  await page.reload();
  const send=async text=>{await page.getByRole('textbox',{name:'Сообщение ассистенту'}).fill(text);await page.getByRole('button',{name:'Отправить сообщение',exact:true}).click();};
  const saved=()=>rpc('session_load',{id:session.id});
  try{await body({temp,model,rpc,rawRpc,session,settings,send,saved});}catch(error){await test.info().attach('backend-state',{body:JSON.stringify({requests:model.requests,history:await saved(),session:await rpc('session_get',{id:session.id})},null,2),contentType:'application/json'});throw error;}
 }finally{
  if(configured)await rpc('delete_model_provider',{id:providerId}).catch(()=>{});
  host.kill();await new Promise(resolve=>{if(host.exitCode!==null)resolve();else host.once('exit',resolve);});await model.close();
  if(path.dirname(path.resolve(temp))!==path.resolve(os.tmpdir())||!path.basename(temp).startsWith('minerva-harness-'))throw Error('Unsafe cleanup path');
  await rm(temp,{recursive:true,force:true});
 }
}

for(const desktop of [false,true])test(`session menus pin, rename, fork, archive, restore and copy IDs (${desktop?'desktop':'web'})`,async({page,context},testInfo)=>{
 await context.grantPermissions(['clipboard-read','clipboard-write']);
 await runtime(page,context,async({model,rpc,session,send})=>{
  model.steps.push({text:'Original answer'});
  await page.locator('input[type=file]').setInputFiles({name:'note.txt',mimeType:'text/plain',buffer:Buffer.from('hello')});
  await send('Session actions');await expect(page.locator('.message.assistant')).toContainText('Original answer');
  const original=(await rpc('session_load',{id:session.id}));
  const menu=title=>page.getByRole('button',{name:`Меню диалога — ${title}`,exact:true});
  await menu('Session actions').click();
  await page.screenshot({path:testInfo.outputPath('session-menu.png')});
  await page.getByRole('menuitem',{name:'Закрепить диалог',exact:true}).click();
  await expect.poll(async()=>(await rpc('workspaces.list')).some(w=>w.pinnedSessionIds.includes(session.id))).toBe(true);
  await menu('Session actions').click();await page.getByRole('menuitem',{name:'Переименовать',exact:true}).click();
  await page.getByLabel('Название диалога',{exact:true}).fill('Renamed chat');await page.getByRole('button',{name:'Сохранить',exact:true}).click();
  await menu('Renamed chat').click();await page.getByRole('menuitem',{name:'Копировать ID диалога',exact:true}).click();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(session.id);
  await menu('Renamed chat').click();await page.getByRole('menuitem',{name:'Создать ветку',exact:true}).click();
  await expect(page.locator('.breadcrumbs strong')).toHaveText('Renamed chat — ветка');
  await expect(page.locator('.message.assistant')).toContainText('Original answer');
  const fork=(await rpc('session_list')).find(item=>item.title==='Renamed chat — ветка');
  expect(fork.id).not.toBe(session.id);
  const ref=original.flatMap(item=>item.parts).find(part=>part.kind==='file').data;
  expect((await rpc('attachments.get',{sessionId:fork.id,id:ref.id})).data).toBe(Buffer.from('hello').toString('base64'));
  model.steps.push({text:'Fork answer'});await send('Continue fork');
  await expect(page.locator('.messages')).toContainText('Fork answer');expect(await rpc('session_load',{id:session.id})).toEqual(original);
  await menu(fork.title).click();await page.getByRole('menuitem',{name:'Архивировать диалог',exact:true}).click();
  await expect(menu(fork.title)).toHaveCount(0);
  await page.getByRole('button',{name:'Архив диалогов',exact:true}).click();await menu(fork.title).click();
  await page.getByRole('menuitem',{name:'Восстановить диалог',exact:true}).click();
  await page.getByRole('button',{name:'Вернуться к диалогам',exact:true}).click();await expect(menu(fork.title)).toBeAttached();
  await page.reload();await expect(menu('Renamed chat')).toBeAttached();
 },desktop);
});

for(const desktop of [false,true])test(`switching chats keeps a background run alive and allows another chat to send (${desktop?'desktop':'web'})`,async({page,context})=>{
 await runtime(page,context,async({model,rpc,session,send})=>{
  model.steps.push({text:'First background result',prefix:'First is working',delay:5000},{text:'Second result'});
  await send('First task');
  await expect(page.locator('.message.assistant')).toContainText('First is working');
  await page.getByRole('button',{name:'Новая сессия',exact:true}).click();
  await expect(page.locator('.message')).toHaveCount(0);
  await expect(page.getByRole('textbox',{name:'Сообщение ассистенту'})).toBeEnabled();
  await send('Second task');
  await expect(page.locator('.message.assistant')).toContainText('Second result');
  const firstAssistant=(await rpc('session_load',{id:session.id})).find(item=>item.role==='assistant');
  expect((await rpc('runs.get',{sessionId:session.id,runId:firstAssistant.id})).status).toBe('running');
  await page.locator('.workspace-sessions button').filter({hasText:'First task'}).click();
  await expect(page.locator('.message.assistant')).toContainText('First is working');
  await expect(page.locator('.message.assistant')).toContainText('First background result');
  expect((await rpc('session_load',{id:session.id})).some(item=>item.role==='assistant')).toBe(true);
 },desktop);
});

test('real native tools, exact questions, approval allow/deny/cancel and durable execution cards',async({page,context})=>{
 await runtime(page,context,async({temp,model,rpc,session,send,saved})=>{
  model.steps.push({tool:'read_file',id:'read-exact',args:{path:'fixture.txt'},prefix:'Reading the real fixture now.',delay:600},{tool:'load_skill',id:'skill-exact',args:{name:'fixture'}},{tool:'patch_file',id:'patch-allow',args:{path:'fixture.txt',oldText:'original',newText:'changed'}},{tool:'ask_user',id:'question-exact',args:{prompt:'Which exact target?',options:['release','main']}},{text:'Native workflow complete'});
  await send('native workflow');
  await expect(page.locator('.message.assistant')).toContainText('Inspecting fixture workspace');
  await expect(page.locator('.message.assistant')).toContainText('Reading the real fixture now.');
  const pending=page.locator('.harness-interaction');await expect(pending).toContainText('patch_file');
  expect(await readFile(path.join(temp,'fixture.txt'),'utf8')).toBe('original fixture value');
  await expect(page.locator('[data-call-id="patch-allow"]')).toHaveAttribute('data-tool-state','requires-approval');
  await writeFile(path.join(temp,'AGENTS.md'),'REFRESHED-DIRECTIVE: the approval is still for one call.');
  await page.reload();await expect(pending).toContainText('patch_file');await expect(page.locator('.message-interrupted')).toHaveCount(0);
  await page.getByRole('button',{name:'Разрешить этот вызов'}).click();
  await expect(pending).toContainText('Which exact target?');await page.getByRole('radio',{name:'release',exact:true}).check();await page.getByRole('button',{name:'Отправить ответ',exact:true}).click();
  await expect(page.locator('.message.assistant')).toContainText('Native workflow complete');
  expect(await readFile(path.join(temp,'fixture.txt'),'utf8')).toBe('changed fixture value');
  expect(JSON.stringify(model.requests[0])).toContain('WORKSPACE-DIRECTIVE');expect(JSON.stringify(model.requests[0])).not.toContain('SKILL-DIRECTIVE');
  const afterRead=model.requests[1].messages.find(m=>m.role==='tool'&&m.tool_call_id==='read-exact');expect(afterRead.content).toContain('original fixture value');
  expect(JSON.stringify(model.requests[2])).toContain('SKILL-DIRECTIVE');
  expect(JSON.stringify(model.requests[3])).toContain('REFRESHED-DIRECTIVE');
  expect(model.requests[4].messages.find(m=>m.role==='tool'&&m.tool_call_id==='question-exact').content).toContain('release');
  const history=await saved();await page.reload();await expect(page.locator('.message.assistant')).toContainText('Native workflow complete');expect(await saved()).toEqual(history);expect(model.requests).toHaveLength(5);
  await expect(page.locator('[data-call-id="read-exact"]')).toHaveAttribute('data-tool-state','completed');
  await page.locator('[data-call-id="read-exact"] > summary').click();await expect(page.getByLabel('Содержимое файла')).toHaveText('original fixture value');
  await page.locator('[data-call-id="patch-allow"] > summary').click();await expect(page.getByLabel('Изменения файла')).toContainText('-original\n+changed');
  model.steps.push({tool:'patch_file',id:'patch-deny',args:{path:'fixture.txt',oldText:'changed',newText:'denied'}},{text:'Denied edit acknowledged'});await send('deny edit');await expect(pending).toContainText('patch_file');await page.getByRole('button',{name:'Отклонить',exact:true}).click();await expect(page.locator('.message.assistant').last()).toContainText('Denied edit acknowledged');
  expect(JSON.stringify(model.requests.at(-1))).toContain('denied');expect(await readFile(path.join(temp,'fixture.txt'),'utf8')).toBe('changed fixture value');
  model.steps.push({tool:'patch_file',id:'patch-cancel',args:{path:'fixture.txt',oldText:'changed',newText:'cancelled'}});await send('cancel pending edit');await expect(pending).toContainText('patch_file');const [interaction]=await rpc('interactions.list',{sessionId:session.id});await page.reload();await expect(pending).toContainText('patch_file');await page.getByRole('button',{name:'Остановить ответ'}).click();
  await expect.poll(()=>rpc('interactions.list',{sessionId:session.id})).toEqual([]);expect(await rpc('interactions.answer',{sessionId:session.id,id:interaction.id,decision:'allow',answer:null})).toBe(false);
  expect(await readFile(path.join(temp,'fixture.txt'),'utf8')).toBe('changed fixture value');await page.reload();await expect(page.locator('[data-call-id="patch-cancel"]')).toHaveAttribute('data-tool-state','cancelled');
 });
});

test('a recovered running checkpoint keeps following the server through completion',async({page,context})=>{
 await runtime(page,context,async({model,send})=>{
  model.steps.push({prefix:'Checkpoint before reload',delay:1800,text:'Recovered final answer'});await send('recover this run');
  await expect(page.locator('.message.assistant')).toContainText('Checkpoint before reload');await page.reload();
  await expect(page.locator('.message.assistant')).toContainText('Recovered final answer');
  await expect(page.getByRole('button',{name:'Отправить сообщение',exact:true})).toBeVisible();await expect(page.locator('.message-interrupted')).toHaveCount(0);expect(model.requests).toHaveLength(1);
 });
});

test('real shell stdout stays running until one terminal result and survives reload',async({page,context})=>{
 await runtime(page,context,async({temp,model,rpc,session,settings,send,saved})=>{
  await rpc('workspace.save',{sessionId:session.id,settings:{...settings,permissionPolicy:'allow'}});
  const command=process.platform==='win32'
   ? "Add-Content -LiteralPath shell-effects.txt -Value 'once'; [Console]::WriteLine('shell-live-marker'); [Console]::Out.Flush(); Start-Sleep -Seconds 4; [Console]::WriteLine('shell-final-marker')"
   : "printf 'once\\n' >> shell-effects.txt; printf 'shell-live-marker\\n'; sleep 4; printf 'shell-final-marker\\n'";
  model.steps.push({tool:'shell',id:'shell-stream-exact',args:{command}},{text:'Shell workflow complete'});
  await send('stream shell output');const card=page.locator('[data-call-id="shell-stream-exact"]');await card.locator('summary').first().click();
  await expect(card.getByLabel('Стандартный вывод')).toContainText('shell-live-marker');
  await expect(card).toHaveAttribute('data-tool-state','running');await expect(card.getByLabel('Стандартный вывод')).not.toContainText('shell-final-marker');
  expect((await saved()).flatMap(message=>message.parts).filter(part=>part.kind==='tool-result'&&part.data.callId==='shell-stream-exact')).toHaveLength(0);expect(model.requests).toHaveLength(1);
  await expect(page.locator('.message.assistant')).toContainText('Shell workflow complete',{timeout:10000});await expect(card).toHaveAttribute('data-tool-state','completed');
  await expect(card.getByLabel('Стандартный вывод')).toContainText('shell-live-marker');await expect(card.getByLabel('Стандартный вывод')).toContainText('shell-final-marker');await expect(card).toContainText('Код выхода: 0');
  const history=await saved(),results=history.flatMap(message=>message.parts).filter(part=>part.kind==='tool-result'&&part.data.callId==='shell-stream-exact');expect(results).toHaveLength(1);expect(JSON.stringify(results[0])).toContain('shell-final-marker');
  await page.reload();await expect(card).toHaveAttribute('data-tool-state','completed');await card.locator('summary').first().click();await expect(card.getByLabel('Стандартный вывод')).toContainText('shell-final-marker');expect(await saved()).toEqual(history);expect(model.requests).toHaveLength(2);expect((await readFile(path.join(temp,'shell-effects.txt'),'utf8')).trim()).toBe('once');
 });
});

test('arbitrary files persist, download and are read or copied by session tools',async({page,context})=>{
 await runtime(page,context,async({model,rpc,rawRpc,session,saved,send,temp,settings})=>{
  const binary=Buffer.from([0,255,128,42]),text='attachment-text-marker';
  const chooser=page.waitForEvent('filechooser');await page.getByRole('button',{name:'Добавить вложение'}).click();
  await page.screenshot({path:'agent-artifacts/composer-menu.png'});
  await page.getByRole('menuitem',{name:'Прикрепить файл'}).click();
  await (await chooser).setFiles([{name:'archive.custom',mimeType:'application/octet-stream',buffer:binary},{name:'notes.txt',mimeType:'text/plain',buffer:Buffer.from(text)}]);
  model.steps.push({text:'Files attached'});await page.getByRole('button',{name:'Отправить сообщение',exact:true}).click();await expect(page.locator('.message.assistant')).toContainText('Files attached');
  const files=(await saved()).flatMap(m=>m.parts).filter(p=>p.kind==='file');expect(files).toHaveLength(2);
  const archive=files.find(p=>p.data.name==='archive.custom').data,notes=files.find(p=>p.data.name==='notes.txt').data;
  expect(JSON.stringify(model.requests[0])).toContain('read_attachment');expect(JSON.stringify(model.requests[0])).not.toContain(text);expect(JSON.stringify(model.requests[0])).not.toContain(binary.toString('base64'));
  await page.reload();const downloadEvent=page.waitForEvent('download');await page.getByRole('button',{name:/archive.custom/}).click();const download=await downloadEvent;expect(await readFile(await download.path())).toEqual(binary);
  const other=randomUUID();await rpc('session_create',{id:other});expect((await rawRpc('attachments.get',{sessionId:other,id:archive.id})).ok).toBe(false);
  model.steps.push({tool:'read_attachment',id:'read-att',args:{id:notes.id}},{tool:'save_attachment',id:'save-att',args:{id:archive.id,path:'copied.bin'}},{text:'Files processed'});
  await send('Read notes and copy archive');
  await page.getByRole('button',{name:'Разрешить этот вызов'}).click();
  await expect(page.locator('.message.assistant').last()).toContainText('Files processed');
  expect(await readFile(path.join(temp,'copied.bin'))).toEqual(binary);
  expect(JSON.stringify(model.requests)).toContain(text);
  model.steps.push({tool:'save_attachment',id:'overwrite-att',args:{id:notes.id,path:'copied.bin'}},{text:'Overwrite refused'});
  await send('Try overwrite');await page.getByRole('button',{name:'Разрешить этот вызов'}).click();await expect(page.locator('.message.assistant').last()).toContainText('Overwrite refused');
  expect(await readFile(path.join(temp,'copied.bin'))).toEqual(binary);
  await rpc('workspace.save',{sessionId:session.id,settings:{...settings,permissionPolicy:'read-only'}});
  model.steps.push({tool:'save_attachment',id:'denied-att',args:{id:archive.id,path:'denied.bin'}},{text:'Policy enforced'});
  await send('Try saving with read-only policy');await expect(page.locator('.message.assistant').last()).toContainText('Policy enforced');
  expect(JSON.stringify(model.requests.at(-1))).toContain('read-only policy denies');
 });
});

test('real image chooser persists private references and sends native model image content',async({page,context})=>{
 await runtime(page,context,async({model,rpc,rawRpc,session,saved})=>{
  const png=await page.evaluate(()=>{const canvas=document.createElement('canvas');canvas.width=2;canvas.height=2;const drawing=canvas.getContext('2d');drawing.fillStyle='red';drawing.fillRect(0,0,2,2);return canvas.toDataURL('image/png').split(',')[1];});
  const chooser=page.waitForEvent('filechooser');await page.getByRole('button',{name:'Добавить вложение'}).click();await page.getByRole('menuitem',{name:'Прикрепить файл'}).click();await (await chooser).setFiles({name:'fixture.png',mimeType:'image/png',buffer:Buffer.from(png,'base64')});
  model.steps.push({text:'Image inspected'});await page.getByRole('button',{name:'Отправить сообщение',exact:true}).click();await expect(page.locator('.message.assistant')).toContainText('Image inspected');
  const history=await saved(),image=history.flatMap(message=>message.parts).find(part=>part.kind==='image');expect(image.data.id).toBeTruthy();expect(JSON.stringify(history)).not.toContain(png);
  expect(model.requests[0].messages.flatMap(message=>Array.isArray(message.content)?message.content:[]).find(part=>part.type==='image_url').image_url.url).toBe(`data:image/png;base64,${png}`);
  const gets=[];page.on('request',request=>{if(request.url().endsWith('/api/v1/rpc')&&request.postDataJSON()?.method==='attachments.get')gets.push(request.postDataJSON().params);});await page.reload();await expect(page.locator('img[alt="fixture.png"]')).toBeVisible();expect(gets).toContainEqual({sessionId:session.id,id:image.data.id});
  const other=randomUUID();await rpc('session_create',{id:other});const denied=await rawRpc('attachments.get',{sessionId:other,id:image.data.id});expect(denied.ok).toBe(false);expect(JSON.stringify(denied)).not.toContain(png);expect((await rpc('attachments.get',{sessionId:session.id,id:image.data.id})).data).toBe(png);
 });
});

test('manual and automatic compaction call the selected model without deleting history',async({page,context})=>{
 test.setTimeout(90000);
 await runtime(page,context,async({model,rpc,session,settings,send,saved})=>{
  for(let index=0;index<4;index++){model.steps.push({text:`turn-${index} ${'history '.repeat(800)}`});await send(`question-${index}`);await expect(page.locator('.message.assistant').last()).toContainText(`turn-${index}`);await expect(page.getByRole('button',{name:'Отправить сообщение',exact:true})).toBeVisible();}
  const before=await saved();await page.getByRole('button',{name:'Сжать контекст',exact:true}).click();await expect.poll(()=>model.summaries).toBe(1);await expect.poll(async()=>(await rpc('context.status',{sessionId:session.id})).estimatedTokens).toBeLessThan(6000);expect(await saved()).toEqual(before);
  model.steps.push({text:'after manual summary'});await send('continue after manual');await expect(page.locator('.message.assistant').last()).toContainText('after manual summary');expect(JSON.stringify(model.requests.at(-1))).toContain('SUMMARY-1');expect(JSON.stringify(model.requests.at(-1))).not.toContain('question-0');
  const budget=await rpc('context.status',{sessionId:session.id});await rpc('workspace.save',{sessionId:session.id,settings:{...settings,contextWindow:Math.max(2048,Math.ceil(budget.estimatedTokens*1.1))}});
  const original=await saved();model.steps.push({text:'after automatic summary'});await send('trigger automatic compaction');await expect(page.locator('.message.assistant').last()).toContainText('after automatic summary');expect(model.summaries).toBe(2);expect(JSON.stringify(model.requests.at(-1))).toContain('SUMMARY-2');
  const after=await saved();expect(after.slice(0,original.length)).toEqual(original);await page.reload();await expect(page.locator('.message.assistant').first()).toContainText('turn-0');
 });
});

test('real MCP process registers model tools and executes only after user approval',async({page,context})=>{
 await runtime(page,context,async({temp,model,rpc,session,send})=>{
  const log=path.join(temp,'mcp.log');await rpc('mcp.save',{server:{id:'fixture',enabled:true,transport:{kind:'stdio',command:process.execPath,args:[path.resolve('plugins/mh-mcp/src/fixture.cjs'),'stdio',log]}}});
  await expect.poll(async()=>(await rpc('mcp.list'))[0]?.state).toBe('connected');
  const tools=await rpc('tools.list',{sessionId:session.id}),tool=tools.find(tool=>tool.name.includes('fixture')&&tool.name.endsWith('echo'));expect(tool).toBeTruthy();
  model.steps.push({tool:tool.name,id:'mcp-exact',args:{text:'actual MCP process result'}},{text:'MCP workflow complete'});await send('call MCP fixture');await expect(page.locator('.harness-interaction')).toContainText(tool.name);await expect(readFile(log,'utf8').catch(()=>'' )).resolves.toBe('');await page.getByRole('button',{name:'Разрешить этот вызов'}).click();await expect(page.locator('.message.assistant')).toContainText('MCP workflow complete');
  expect(await readFile(log,'utf8')).toBe('echo\n');expect(model.requests[0].tools.some(value=>value.function.name===tool.name)).toBe(true);expect(model.requests[1].messages.find(message=>message.tool_call_id==='mcp-exact').content).toContain('actual MCP process result');await page.reload();expect(await readFile(log,'utf8')).toBe('echo\n');
 });
});
