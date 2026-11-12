import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {createServer} from 'node:http';
import {mkdtemp,rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {randomUUID} from 'node:crypto';

test('real Rust Host persists chat across reload and observes a run after the tab closes',async({page,context})=>{
 const temp=await mkdtemp(path.join(os.tmpdir(),'minerva-web-'));
 const providerId=`e2e${randomUUID().replaceAll('-','')}`;
 let modelCalls=0,providerDelay=0;
 const provider=createServer(async(req,res)=>{
  let body='';for await(const chunk of req)body+=chunk;
  if(req.url!=='/v1/chat/completions'){res.writeHead(404);res.end();return;}
  modelCalls++;
  const content=JSON.parse(body);
  const user=content.messages.filter(m=>m.role==='user').at(-1)?.content;
  if(providerDelay)await new Promise(resolve=>setTimeout(resolve,providerDelay));
  expect(content.stream).toBe(true);
  const text=`Saved reply ${modelCalls}: ${typeof user==='string'?user:'question'}`;
  const chunk=(delta,finish_reason=null)=>({id:'fixture',object:'chat.completion.chunk',created:1,model:'fixture-model',choices:[{index:0,delta,finish_reason}]});
  res.setHeader('Content-Type','text/event-stream');
  res.write(`data: ${JSON.stringify(chunk({role:'assistant',content:text}))}\n\n`);
  res.write(`data: ${JSON.stringify({...chunk({},'stop'),usage:{prompt_tokens:1,completion_tokens:1,total_tokens:2}})}\n\n`);
  res.end('data: [DONE]\n\n');
 });
 await new Promise(resolve=>provider.listen(0,'127.0.0.1',resolve));
 const providerUrl=`http://127.0.0.1:${provider.address().port}/v1`;
 const executable=path.resolve('target/debug',process.platform==='win32'?'minerva-server.exe':'minerva-server');
 const env={...process.env};
 // Exercise only fixture HTTP, never a paid provider.
 if(process.platform==='win32')env.PATH=`${env.SystemRoot};${env.SystemRoot}\\System32`;
 else env.PATH='/usr/bin:/bin';
 for(const name of ['OPENAI_API_KEY','ANTHROPIC_API_KEY','DEEPSEEK_API_KEY'])delete env[name];
 const host=spawn(executable,['--bind','127.0.0.1:0','--data-dir',path.join(temp,'data'),'--config-dir',path.join(temp,'config'),'--web-dist',path.resolve('dist')],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
 const launch=await new Promise((resolve,reject)=>{
  const timeout=setTimeout(()=>reject(Error('Rust Host did not become ready')),15000);
  let output='',errors='';
  host.stderr.on('data',chunk=>{errors+=chunk;});
  host.once('error',error=>{clearTimeout(timeout);reject(error);});
  host.once('exit',code=>{clearTimeout(timeout);reject(Error(`Rust Host exited ${code}: ${errors}`));});
  host.stdout.on('data',chunk=>{output+=chunk;const match=output.match(/http:\/\/127\.0\.0\.1:\d+\/#minerva-token=[a-f0-9]+/);if(match){clearTimeout(timeout);resolve(match[0]);}});
 });
 const origin=new URL(launch).origin;
 const rpc=async(method,params={})=>{
  const response=await context.request.get(`${origin}/api/v1/bootstrap`);
  const bootstrap=await response.json();
  const result=await context.request.post(`${origin}/api/v1/rpc`,{headers:{Origin:origin,'X-Minerva-CSRF':bootstrap.csrfToken},data:{protocolVersion:1,method,params}});
  const body=await result.json();
  expect(body.ok,JSON.stringify(body.error)).toBe(true);return body.value;
 };
 let configured=false;
 try{
  await page.goto(launch);
  await expect(page.getByRole('button',{name:'Новая сессия'})).toBeVisible();
  await expect.poll(()=>new URL(page.url()).hash).toBe('');
  await rpc('save_model_provider',{provider:{id:providerId,name:'Fixture provider',baseUrl:providerUrl,models:['fixture-model','fixture-other'],apiKey:'disposable-fixture-key',isNew:true,protocol:'openai-chat'}});configured=true;
  const [session]=await rpc('session_list');
  await rpc('session_set_model',{id:session.id,model:`api:${providerId}/fixture-model`});
  await page.reload();
  await page.getByRole('textbox',{name:'Сообщение ассистенту'}).fill('first question');
  await page.getByRole('button',{name:'Отправить сообщение',exact:true}).click();
  await expect(page.locator('.message.assistant')).toContainText('Saved reply 1');
  await page.reload();
  await expect(page.locator('.message.assistant')).toContainText('Saved reply 1');
  expect(modelCalls).toBe(1);
  const second=await context.newPage();await second.goto(origin);
  await expect(second.getByRole('button',{name:'Новая сессия'})).toBeVisible();
  await rpc('session_set_model',{id:session.id,model:`api:${providerId}/fixture-other`});
  await expect(page.getByRole('button',{name:'Модель',exact:true})).toContainText('fixture-other');
  await expect(second.getByRole('button',{name:'Модель',exact:true})).toContainText('fixture-other');
  await second.close();

  providerDelay=2500;
  await page.getByRole('textbox',{name:'Сообщение ассистенту'}).fill('survive tab close');
  await page.getByRole('button',{name:'Отправить сообщение',exact:true}).click();
  await expect.poll(()=>modelCalls).toBe(2);
  await page.close();
  const restored=await context.newPage();await restored.goto(origin);
  await expect(restored.getByRole('status',{exact:false}).filter({hasText:'Задача выполняется на сервере'})).toBeVisible();
  await expect(restored.getByRole('textbox',{name:'Сообщение ассистенту'})).toBeDisabled();
  await expect(restored.locator('.message.assistant').last()).toContainText('Saved reply 2');
  expect(modelCalls).toBe(2);

  providerDelay=3000;
  const cancelledStart=restored.waitForResponse(response=>response.url()===`${origin}/api/v1/rpc`&&response.request().postDataJSON()?.method==='runs.start');
  await restored.getByRole('textbox',{name:'Сообщение ассистенту'}).fill('cancel question');
  await restored.getByRole('button',{name:'Отправить сообщение',exact:true}).click();
  await expect.poll(()=>modelCalls).toBe(3);
  const cancelledHandle=(await (await cancelledStart).json()).value;
  await restored.getByRole('button',{name:'Остановить ответ'}).click();
  await expect(restored.getByRole('button',{name:'Отправить сообщение',exact:true})).toBeVisible();
  // No checkpoint exists yet; cancellation must not manufacture an assistant message.
  await expect.poll(async()=>(await rpc('runs.get',cancelledHandle)).status).toBe('cancelled');
  await restored.reload();
  await expect(restored.locator('.message.assistant')).toHaveCount(2);
  await restored.setViewportSize({width:390,height:844});
  await restored.getByRole('button',{name:'Настройки',exact:true}).click();
  await expect(restored.locator('.settings-screen')).toBeVisible();
  expect(await restored.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await restored.getByRole('button',{name:'Боковая панель',exact:true}).click();
  await expect(restored.getByRole('dialog')).toBeVisible();
  await restored.getByRole('button',{name:'Серверы MCP',exact:true}).click();
  await expect(restored.getByRole('dialog')).toHaveCount(0);
  await expect(restored.getByRole('button',{name:'Добавить сервер MCP'})).toBeVisible();
 }finally{
  if(configured)await rpc('delete_model_provider',{id:providerId}).catch(()=>{});
  host.kill();await new Promise(resolve=>{if(host.exitCode!==null)resolve();else host.once('exit',resolve);});
  provider.closeAllConnections();await new Promise(resolve=>provider.close(resolve));
  const resolved=path.resolve(temp);
  if(path.dirname(resolved)!==path.resolve(os.tmpdir())||!path.basename(resolved).startsWith('minerva-web-'))throw Error('Unexpected test cleanup path');
  await rm(resolved,{recursive:true,force:true});
 }
});
