import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,writeFile,copyFile,rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';

test('Host isolates broken packages, slow calls and bad catalogues; UI follows crashes',async({page,context})=>{
  const temp=await mkdtemp(path.join(os.tmpdir(),'minerva-isolation-'));
  const config=path.join(temp,'config'),suffix=process.platform==='win32'?'.exe':'';
  const fixture=path.resolve(`target/debug/plugin-fixture${suffix}`);
  let host;
  try{
    for(const id of ['healthy','bad-catalogue','slow']){
      const source=path.join(temp,id);await mkdir(source);
      await copyFile(fixture,path.join(source,`backend${suffix}`));
      const manifest={id,name:id,version:'1',apiVersion:1,backend:{kind:'process',entry:`backend${suffix}`},capabilities:id==='slow'?[]:['model-engine']};
      if(id==='healthy'){
        manifest.ui={entry:'client.js',sdkVersion:2};
        await writeFile(path.join(source,'client.js'),`const React=require('react');exports.sdkVersion=2;exports.apply=ctx=>ctx.slots.register('settings.pages',{entryId:'plugin:healthy',component:p=>p.placement==='navigation'?React.createElement('button',{onClick:()=>p.onSelectPage('plugin:healthy')},'Healthy settings'):React.createElement('h2',null,'Healthy panel')});`);
      }
      await writeFile(path.join(source,'plugin.json'),JSON.stringify(manifest));
      await writeFile(path.join(source,'catalog.json'),JSON.stringify(id==='healthy'?{models:[{id:'fixture',name:'Fixture',isDefault:true}],loading:false,error:null}:{models:[{id:'',name:'Invalid',isDefault:false}],loading:false,error:null}));
      await new Promise((resolve,reject)=>{
        const child=spawn(fixture,['--install',source,config],{windowsHide:true,stdio:'pipe'});
        let error='';child.stderr.on('data',chunk=>{error+=chunk;});child.on('error',reject);child.on('exit',code=>code===0?resolve():reject(Error(error)));
      });
    }
    const broken=path.join(config,'plugins','invalid');await mkdir(broken);await writeFile(path.join(broken,'plugin.json'),'{}');
    host=spawn(path.resolve(`target/debug/minerva-server${suffix}`),['--bind','127.0.0.1:0','--data-dir',path.join(temp,'data'),'--config-dir',config,'--web-dist',path.resolve('dist')],{windowsHide:true,stdio:['ignore','pipe','pipe']});
    const launch=await new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>reject(Error('Host startup timeout')),15000);let output='',errors='';
      host.stderr.on('data',chunk=>{errors+=chunk;});
      host.on('error',error=>{clearTimeout(timer);reject(error);});host.on('exit',code=>{clearTimeout(timer);reject(Error(`Host exit ${code}: ${errors}`));});
      host.stdout.on('data',chunk=>{output+=chunk;const found=output.match(/http:\/\/127\.0\.0\.1:\d+\/#minerva-token=[a-f0-9]+/);if(found){clearTimeout(timer);resolve(found[0]);}});
    });
    const origin=new URL(launch).origin;
    await page.goto(launch);await expect(page.locator('.settings-open')).toBeVisible();
    const bootstrap=await (await context.request.get(`${origin}/api/v1/bootstrap`)).json();
    const raw=async(method,params={})=>(await context.request.post(`${origin}/api/v1/rpc`,{headers:{Origin:origin,'X-Minerva-CSRF':bootstrap.csrfToken},data:{protocolVersion:1,method,params}})).json();
    const rpc=async(method,params)=>{const body=await raw(method,params);expect(body.ok,JSON.stringify(body.error)).toBe(true);return body.value;};
    expect((await rpc('list_plugins')).find(p=>p.id==='invalid')).toMatchObject({enabled:false,error:expect.any(String)});
    for(const id of ['healthy','bad-catalogue','slow'])await rpc('set_plugin_enabled',{id,enabled:true});
    expect((await rpc('plugins.models')).map(model=>model.id)).toEqual(['plugin:healthy/fixture']);
    expect((await rpc('list_plugins')).find(p=>p.id==='bad-catalogue').error).toMatch(/model ID/);
    let slowFinished=false;
    const slow=raw('plugins.call',{id:'slow',method:'test.hang',params:{}}).then(result=>{slowFinished=true;return result;});
    await new Promise(resolve=>setTimeout(resolve,100));
    const started=Date.now();
    await rpc('plugins.call',{id:'healthy',method:'review.summary',params:{}});
    await rpc('list_plugins');
    expect(slowFinished).toBe(false);expect(Date.now()-started).toBeLessThan(2000);
    expect((await slow).ok).toBe(false);
    await page.locator('.settings-open').click();
    const dialog=page.locator('.navigation-shell');
    await dialog.getByRole('button',{name:'Healthy settings'}).click();
    await expect(dialog.getByRole('heading',{name:'Healthy panel'})).toBeVisible();
    await rpc('plugins.call',{id:'healthy',method:'test.exit',params:{}});
    await expect(dialog.getByRole('button',{name:'Healthy settings'})).toHaveCount(0,{timeout:10000});
    await expect(dialog.getByRole('button',{name:'Модели',exact:true})).toHaveAttribute('aria-current','page');
    await page.getByRole('button',{name:'Чаты',exact:true}).click();
    await page.getByRole('button',{name:'Плагины',exact:true}).click();
    const row=page.locator('.plugin-row').filter({hasText:'healthy'});
    await expect(row.getByRole('alert')).toBeVisible();
    await row.getByRole('button',{name:'Повторить запуск'}).click();
    await expect(row.getByRole('button',{name:'Отключить',exact:true})).toBeVisible();
    await rpc('set_plugin_enabled',{id:'slow',enabled:false});
    const master=page.getByRole('switch',{name:'Несистемные плагины'});
    await expect(master).toBeChecked();
    await master.click();
    await expect(master).not.toBeChecked();
    let paused=await rpc('list_plugins');
    expect(paused.filter(p=>p.canToggle).every(p=>!p.enabled)).toBe(true);
    expect(paused.filter(p=>!p.canToggle).every(p=>p.enabled)).toBe(true);
    expect(paused.find(p=>p.id==='healthy').requestedEnabled).toBe(true);
    expect(paused.find(p=>p.id==='slow').requestedEnabled).toBe(false);
    await page.locator('.settings-open').click();
    await expect(dialog.getByRole('button',{name:'Healthy settings'})).toHaveCount(0);
    await page.getByRole('button',{name:'Плагины',exact:true}).click();
    await master.click();
    await expect(master).toBeChecked();
    paused=await rpc('list_plugins');
    expect(paused.find(p=>p.id==='healthy').enabled).toBe(true);
    expect(paused.find(p=>p.id==='slow').enabled).toBe(false);
    await page.locator('.settings-open').click();
    await expect(dialog.getByRole('button',{name:'Healthy settings'})).toBeVisible();
  }finally{
    if(host){host.kill();await new Promise(resolve=>{if(host.exitCode!==null)resolve();else host.once('exit',resolve);});}
    const resolved=path.resolve(temp);if(path.dirname(resolved)!==path.resolve(os.tmpdir())||!path.basename(resolved).startsWith('minerva-isolation-'))throw Error('Unexpected cleanup path');
    await rm(resolved,{recursive:true,force:true});
  }
});
