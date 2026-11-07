import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,writeFile,readFile,rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {harnessModel} from '../fixtures/harness-model.mjs';

async function runtime(page,context,body){
 const temp=await mkdtemp(path.join(os.tmpdir(),'minerva-harness-')),model=await harnessModel(),providerId=`harness${randomUUID().replaceAll('-','')}`;
 await mkdir(path.join(temp,'.agents/skills/fixture'),{recursive:true});
 await writeFile(path.join(temp,'AGENTS.md'),'WORKSPACE-DIRECTIVE: inspect fixture.txt before editing.');
 await writeFile(path.join(temp,'.agents/skills/fixture/SKILL.md'),'---\ndescription: fixture skill\n---\nSKILL-DIRECTIVE: keep exact fixture values.');
 await writeFile(path.join(temp,'fixture.txt'),'original fixture value');
 const env={...process.env};for(const key of ['OPENAI_API_KEY','ANTHROPIC_API_KEY','DEEPSEEK_API_KEY'])delete env[key];
 const host=spawn(path.resolve('target/debug',process.platform==='win32'?'minerva-server.exe':'minerva-server'),['--bind','127.0.0.1:0','--data-dir',path.join(temp,'data'),'--config-dir',path.join(temp,'config'),'--web-dist',path.resolve('dist')],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
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
  const [session]=await rpc('session_list');
  await rpc('session_set_model',{id:session.id,model:`api:${providerId}/fixture`});
  const settings={cwd:temp,permissionPolicy:'ask',contextWindow:32768,autoCompact:true};
  await rpc('workspace.save',{sessionId:session.id,settings});await page.reload();
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

test('real image chooser persists private references and sends native model image content',async({page,context})=>{
 await runtime(page,context,async({model,rpc,rawRpc,session,saved})=>{
  const png=await page.evaluate(()=>{const canvas=document.createElement('canvas');canvas.width=2;canvas.height=2;const drawing=canvas.getContext('2d');drawing.fillStyle='red';drawing.fillRect(0,0,2,2);return canvas.toDataURL('image/png').split(',')[1];});
  const chooser=page.waitForEvent('filechooser');await page.getByRole('button',{name:'Прикрепить изображение'}).click();await (await chooser).setFiles({name:'fixture.png',mimeType:'image/png',buffer:Buffer.from(png,'base64')});
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
  const log=path.join(temp,'mcp.log');await rpc('mcp.save',{server:{id:'fixture',enabled:true,transport:{kind:'stdio',command:process.execPath,args:[path.resolve('crates/runtime/src/mcp/fixture.cjs'),'stdio',log]}}});
  await expect.poll(async()=>(await rpc('mcp.list'))[0]?.state).toBe('connected');
  const tools=await rpc('tools.list',{sessionId:session.id}),tool=tools.find(tool=>tool.name.includes('fixture')&&tool.name.endsWith('echo'));expect(tool).toBeTruthy();
  model.steps.push({tool:tool.name,id:'mcp-exact',args:{text:'actual MCP process result'}},{text:'MCP workflow complete'});await send('call MCP fixture');await expect(page.locator('.harness-interaction')).toContainText(tool.name);await expect(readFile(log,'utf8').catch(()=>'' )).resolves.toBe('');await page.getByRole('button',{name:'Разрешить этот вызов'}).click();await expect(page.locator('.message.assistant')).toContainText('MCP workflow complete');
  expect(await readFile(log,'utf8')).toBe('echo\n');expect(model.requests[0].tools.some(value=>value.function.name===tool.name)).toBe(true);expect(model.requests[1].messages.find(message=>message.tool_call_id==='mcp-exact').content).toContain('actual MCP process result');await page.reload();expect(await readFile(log,'utf8')).toBe('echo\n');
 });
});
