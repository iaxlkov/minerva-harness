import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

test('workspace UI plugin creates grouped durable sessions through the real Host',async({page,context},testInfo)=>{
 const temp=await mkdtemp(path.join(os.tmpdir(),'minerva-workspaces-'));
 const projectPath=path.join(temp,'my-project');await mkdir(projectPath);
 const host=spawn(process.env.MINERVA_TEST_SERVER_EXE||path.resolve('target/debug',process.platform==='win32'?'minerva-server.exe':'minerva-server'),['--bind','127.0.0.1:0','--data-dir',path.join(temp,'data'),'--config-dir',path.join(temp,'config'),'--web-dist',path.resolve('dist')],{windowsHide:true,stdio:['ignore','pipe','pipe']});
 try{
  const launch=await new Promise((resolve,reject)=>{
   let stdout='',stderr='';const timeout=setTimeout(()=>reject(Error(stderr||'Host startup timed out')),20000);
   host.stderr.on('data',chunk=>stderr+=chunk);host.once('error',error=>{clearTimeout(timeout);reject(error);});host.once('exit',code=>{clearTimeout(timeout);reject(Error(`Host exited ${code}: ${stderr}`));});
   host.stdout.on('data',chunk=>{stdout+=chunk;const match=stdout.match(/http:\/\/127\.0\.0\.1:\d+\/#minerva-token=[a-f0-9]+/);if(match){clearTimeout(timeout);resolve(match[0]);}});
  });
  await page.goto(launch);
  const origin=new URL(launch).origin;
  const rpc=async(method,params={})=>{
   const boot=await (await context.request.get(`${origin}/api/v1/bootstrap`)).json();
   const result=await (await context.request.post(`${origin}/api/v1/rpc`,{headers:{Origin:origin,'X-Minerva-CSRF':boot.csrfToken},data:{protocolVersion:1,method,params}})).json();
   expect(result.ok,JSON.stringify(result.error)).toBe(true);return result.value;
  };
  await expect(page.getByRole('button',{name:'Default workspace',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Создать workspace',exact:true}).click();
  await page.getByLabel('Папка проекта').fill(projectPath);
  await page.getByRole('button',{name:'Создать',exact:true}).click();
  const group=page.locator('.workspace-group').filter({has:page.getByRole('button',{name:'my-project',exact:true})});
  await expect(group.locator('.workspace-sessions .session-row > .nav-item')).toHaveCount(1);
  const input=page.getByRole('textbox',{name:'Сообщение ассистенту'});
  await input.fill('Unsent draft');
  await page.getByRole('button',{name:'Новая сессия',exact:true}).click();
  await expect(group.locator('.workspace-sessions .session-row > .nav-item')).toHaveCount(1);
  await expect(input).toHaveValue('Unsent draft');
  let project=(await rpc('workspaces.list')).find(item=>item.title==='my-project');
  expect(project.sessionIds).toHaveLength(1);
  // Older empty sessions remain addressable but no longer clutter history.
  await rpc('session_create',{id:'old-empty',workspaceId:project.id});
  for(const sessionId of project.sessionIds)expect((await rpc('workspace.get',{sessionId})).cwd).toBe(project.path);
  await page.reload();
  await expect(group.locator('.workspace-sessions .session-row > .nav-item')).toHaveCount(1);
  await page.getByRole('button',{name:'Новый диалог — Default workspace',exact:true}).click();
  const defaults=page.locator('.workspace-group').filter({has:page.getByRole('button',{name:'Default workspace',exact:true})});
  await expect(defaults.locator('.workspace-sessions .session-row > .nav-item')).toHaveCount(1);
  await expect(group.locator('.workspace-sessions .session-row > .nav-item')).toHaveCount(0);
  expect((await rpc('workspaces.list')).find(item=>item.id==='default').sessionIds).toHaveLength(1);
  // Exercise the desktop bridge contract without opening an interactive OS window in CI.
  await page.evaluate(folder=>{window.__TAURI_INTERNALS__={invoke:async(command,args)=>{
   if(command!=='plugin:dialog|open'||args.options.directory!==true)throw Error('Unexpected native dialog request');
   return folder;
  }};},projectPath);
  await page.getByRole('button',{name:'Создать workspace',exact:true}).click();
  await expect(group.locator('.nav-item.selected')).toHaveCount(1);
  expect((await rpc('workspaces.list')).filter(item=>item.title==='my-project')).toHaveLength(1);
  await expect(group.locator('.workspace-sessions .session-row > .nav-item')).toHaveCount(1);
  await page.evaluate(()=>{window.__TAURI_INTERNALS__.invoke=async()=>null;});
  await page.getByRole('button',{name:'Создать workspace',exact:true}).click();
  await expect(page.getByRole('button',{name:'Создать workspace',exact:true})).toBeEnabled();
  await expect(page.locator('.workspace-group')).toHaveCount(2);
  await page.evaluate(()=>delete window.__TAURI_INTERNALS__);
  await page.getByRole('button',{name:'Настройки',exact:true}).click();
  await expect(page.getByRole('button',{name:'Рабочая папка',exact:true})).toHaveCount(0);
  await page.getByRole('button',{name:'Сессия',exact:true}).click();
  await expect(page.getByLabel('Политика выполнения')).toBeVisible();
  await expect(page.getByLabel('Рабочая папка',{exact:true})).toHaveCount(0);
  await page.getByRole('button',{name:'Закрыть настройки'}).click();
  await page.screenshot({path:testInfo.outputPath('workspaces.png')});
  const triggerBounds=await group.getByRole('button',{name:'Меню workspace — my-project'}).boundingBox();
  await group.getByRole('button',{name:'Меню workspace — my-project'}).click();
  await expect(page.getByRole('menu')).not.toHaveCSS('background-color','rgba(0, 0, 0, 0)');
  const menuBounds=await page.getByRole('menu').boundingBox();
  expect(menuBounds.x).toBeGreaterThanOrEqual(triggerBounds.x-1);
  await page.screenshot({path:testInfo.outputPath('workspace-menu.png')});
  await page.getByRole('menuitem',{name:'Переименовать',exact:true}).click();
  await page.getByLabel('Название workspace').fill('Renamed project');
  await page.getByRole('button',{name:'Сохранить',exact:true}).click();
  const renamed=page.locator('.workspace-group').filter({has:page.getByRole('button',{name:'Renamed project',exact:true})});
  await expect(renamed).toBeVisible();
  await renamed.getByRole('button',{name:'Renamed project',exact:true}).click({button:'right'});
  await page.getByRole('menuitem',{name:'Удалить workspace',exact:true}).click();
  await page.screenshot({path:testInfo.outputPath('workspace-delete-dialog.png')});
  await page.getByRole('button',{name:'Отмена',exact:true}).click();
  await expect(renamed).toBeVisible();
  await renamed.getByRole('button',{name:'Меню workspace — Renamed project'}).click();
  await page.getByRole('menuitem',{name:'Удалить workspace',exact:true}).click();
  await page.getByRole('button',{name:'Удалить workspace',exact:true}).click();
  await expect(renamed).toHaveCount(0);
  await page.reload();await expect(renamed).toHaveCount(0);
  const restored=await rpc('workspaces.create',{path:projectPath});
  expect(restored.id).toBe(project.id);expect(restored.title).toBe('Renamed project');
  expect(restored.sessionIds).toContain(project.sessionIds[0]);
  await page.reload();await expect(renamed).toBeVisible();
 }finally{
  host.kill();await new Promise(resolve=>{if(host.exitCode!==null)resolve();else host.once('exit',resolve);});
  if(path.dirname(path.resolve(temp))!==path.resolve(os.tmpdir())||!path.basename(temp).startsWith('minerva-workspaces-'))throw Error('Unsafe fixture cleanup');
  await rm(temp,{recursive:true,force:true});
 }
});
