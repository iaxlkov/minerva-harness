import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {mkdtemp,rm,mkdir} from 'node:fs/promises';
import path from 'node:path'; import os from 'node:os'; import {randomUUID,createHash} from 'node:crypto';
import {harnessModel} from '../fixtures/harness-model.mjs';

test('learning panels use real host and isolated stores',async({page,context})=>{
 test.setTimeout(120000); const temp=await mkdtemp(path.join(os.tmpdir(),'minerva-learning-')); const model=await harnessModel();
 const providerIds=Array.from({length:3},()=>`test-${randomUUID()}`); let rpc;
 const env={...process.env};for(const k of ['OPENAI_API_KEY','ANTHROPIC_API_KEY','DEEPSEEK_API_KEY'])delete env[k];
 const host=spawn(process.env.MINERVA_TEST_SERVER_EXE||path.resolve('target/debug',process.platform==='win32'?'minerva-server.exe':'minerva-server'),['--bind','127.0.0.1:0','--data-dir',path.join(temp,'data'),'--config-dir',path.join(temp,'config'),'--web-dist',path.resolve('dist')],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
 try {
  const url=await new Promise((resolve,reject)=>{let out='',err='';const timer=setTimeout(()=>reject(Error(err||'startup timeout')),25000);host.stderr.on('data',c=>err+=c);host.on('error',reject);host.on('exit',code=>reject(Error(`server exited ${code}: ${err}`)));host.stdout.on('data',c=>{out+=c;const m=out.match(/http:\/\/127\.0\.0\.1:\d+\/#minerva-token=[a-f0-9]+/);if(m){clearTimeout(timer);resolve(m[0]);}});});
  const origin=new URL(url).origin;await page.goto(url); const boot=await (await context.request.get(`${origin}/api/v1/bootstrap`)).json();
  rpc=async(method,params={})=>{const r=await(await context.request.post(`${origin}/api/v1/rpc`,{headers:{Origin:origin,'X-Minerva-CSRF':boot.csrfToken},data:{protocolVersion:1,method,params}})).json();expect(r.ok,JSON.stringify(r.error)).toBe(true);return r.value;};
  const project=await rpc('workspaces.create',{path:temp});const sessionId=randomUUID();await rpc('session_create',{id:sessionId,workspaceId:project.id});
  for(const id of ['skills','session-query','memory','experiments','experience','improvement','skill-curation']) await rpc('set_plugin_enabled',{id,enabled:true});
  for(const id of providerIds)await rpc('save_model_provider',{provider:{id,name:id,baseUrl:model.url,models:['fixture'],apiKey:'fixture',isNew:true,protocol:'openai-chat'}});
  await page.reload();await page.locator('.settings-open').click();await page.getByRole('button',{name:'Плагины',exact:true}).click();
  const row=page.locator('.plugin-row').filter({has:page.locator('.plugin-name',{hasText:'Память'})});await row.getByRole('button',{name:'Открыть'}).click();await page.getByRole('combobox',{name:'Чат',exact:true}).selectOption(sessionId);
  await page.getByLabel('Содержание').fill('Сохраняй точные имена');await page.getByRole('button',{name:'Сохранить',exact:true}).click();
  await expect(page.locator('.plugin-row p').getByText('Сохраняй точные имена')).toBeVisible();
  await mkdir('agent-artifacts/learning-ui',{recursive:true}); await page.setViewportSize({width:1440,height:900});await page.screenshot({path:'agent-artifacts/learning-ui/desktop.png',fullPage:true});
  await page.setViewportSize({width:390,height:844});await page.screenshot({path:'agent-artifacts/learning-ui/mobile.png',fullPage:true});
  await page.getByRole('button',{name:'Открыть',exact:true}).click();await expect(page.getByRole('heading',{name:'Подробнее'})).toBeVisible();
  await page.getByLabel('Новый текст').fill('Сохраняй точные имена и даты');await page.getByRole('button',{name:'Сохранить с проверкой версии'}).click();await expect(page.locator('.plugin-row p').getByText('Сохраняй точные имена и даты')).toBeVisible();
  await page.getByRole('button',{name:'Открыть',exact:true}).click();await page.getByRole('button',{name:'Забыть'}).click();await expect(page.locator('.plugin-row p').getByText('Сохраняй точные имена и даты')).toHaveCount(0);
  await page.getByRole('button',{name:'Назад'}).click();await row.getByRole('button',{name:'Отключить',exact:true}).click();await row.getByRole('button',{name:'Открыть'}).click();await expect(page.getByText('Включите плагин в списке',{exact:false})).toBeVisible();await page.getByRole('button',{name:'Назад'}).click();await row.getByRole('button',{name:'Включить',exact:true}).click();
  const call=(id,method,payload)=>rpc('plugins.call',{id,method,params:{sessionId,payload}});
  const published=await call('skills','publish',{name:'example',body:'---\nname: example\ndescription: example\n---\nUse exact names.',expectedRevision:null});const loaded=await call('skills','load',{name:'example'});expect(loaded.body).toContain('exact names');expect(loaded.info.revision).toBe(published.revision);
  model.steps.push({text:'Acknowledged'});await rpc('runs.start',{sessionId,requestId:'source-run',parentId:null,message:{id:'source',text:'Use exact names in this skill'},model:`api:${providerIds[0]}/fixture`,options:{toolAllowlist:[]}});await expect.poll(async()=>(await rpc('runs.get',{sessionId,runId:'source-run'})).status,{timeout:30000}).toBe('completed');
  const hits=await call('session-query','search',{query:'exact names'});expect(hits.hits.length).toBeGreaterThan(0);const source=hits.hits[0].source;
  const one={input:'Respond correctly',contains:['correct'],notContains:['wrong']};
  await call('skill-curation','configure',{config:{executorModel:`api:${providerIds[0]}/fixture`,reviewers:[`api:${providerIds[1]}/fixture`,`api:${providerIds[2]}/fixture`],trigger:one,holdouts:[one],maxCalls:6}});
  await call('skill-curation','propose',{proposal:{id:'proposal',scope:{kind:'session',sessionId},actor:'',sources:[source],skill:'example',base_revision:published.revision,candidate:'---\nname: example\ndescription: example\n---\nAlways use exact names.',diff:'Add always.',rationale:'User correction',applicability:'Name tasks',negative_example:'Guess names',validators:[],reviewers:[],state:'draft',evidence:[],published_revision:null}});
  model.steps.push(...Array.from({length:4},()=>({text:'correct'})),{text:'{"approved":true,"reason":"validated"}'},{text:'{"approved":true,"reason":"validated"}'});
  await call('skill-curation','evaluate',{id:'proposal'});await expect.poll(async()=> (await call('skill-curation','get',{id:'proposal'})).state,{timeout:30000}).toBe('ready');
  const result=await call('skill-curation','publish',{id:'proposal'});expect(result.state).toBe('published');expect((await call('skills','load',{name:'example'})).body).toContain('Always');
  await call('skill-curation','rollback',{id:'proposal'});expect((await call('skills','load',{name:'example'})).body).toContain('Use exact names.');
  model.steps.push({text:'baseline done'});const job=await call('experiments','create',{task:'Reply briefly',model:`api:${providerIds[0]}/fixture`,budget:1,k:1});await call('experiments','start',{id:job.id});await expect.poll(async()=>(await call('experiments','get',{id:job.id})).state,{timeout:30000}).toBe('completed');expect((await call('experiments','compare',{id:job.id})).artifact).toBeTruthy();
  const base=createHash('sha256').update('{}').digest('hex');
  for(let i=0;i<8;i++) model.steps.push({get text(){const request=JSON.stringify(model.requests.at(-1));if(request.includes('Stage: GeneralProposal')||request.includes('Stage: DomainProposal')) return JSON.stringify({kind:'proposal',proposal:{targetDeficiency:'precision',scope:request.includes('Stage: GeneralProposal')?'general':'domain',baseDigest:base,componentDiffs:[{component:'systemPrompt',path:'system-prompt.txt',diff:'Be precise'}],hypothesis:'more precision',evidenceRefs:[]}});if(request.includes('Stage: Compose'))return JSON.stringify({kind:'compose',compose:{adoptedDiffs:[],rejectedDiffs:[],reasons:'retain baseline',evidenceRefs:[],resultDigest:'placeholder'}});if(request.includes('Stage: Reflect'))return JSON.stringify({kind:'noEntry'});return 'verified result';}});
  const improved=await call('improvement','start',{task:'Answer precisely',model:`api:${providerIds[0]}/fixture`,budget:8,k:1,mode:'parallel'});await expect.poll(async()=>(await call('improvement','get',{id:improved.id})).state,{timeout:30000}).toBe('completed');const comparison=await call('improvement','compare',{id:improved.id});expect(comparison).toBeTruthy();


 } finally {if(rpc) for(const id of providerIds) await rpc('delete_model_provider',{id}).catch(e=>console.error(`Failed to delete test provider ${id}:`,e));host.kill();await new Promise(resolve=>host.exitCode!==null?resolve():host.once('exit',resolve));await model.close();if(path.dirname(path.resolve(temp))===path.resolve(os.tmpdir())&&path.basename(temp).startsWith('minerva-learning-'))await rm(temp,{recursive:true,force:true});}
});
