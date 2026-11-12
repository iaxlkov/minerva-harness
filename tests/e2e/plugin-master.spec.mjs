import {viteWorkerStub} from './vite-worker-stub.mjs';
import {test,expect} from '@playwright/test';
import {build} from 'esbuild';

test('master switch pauses selection, preserves diagnostics and reports failures',async({page})=>{
 const result=await build({plugins:[viteWorkerStub],stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
  import React from 'react';
  import {createRoot} from 'react-dom/client';
  import {Plugins} from './apps/web/src/pages/plugins/Plugins';
  import {configureClient,createClient} from './packages/client/src/index';
  let optionalEnabled=true;
  let plugins=[
   {id:'plugin-manager',name:'Manager',canToggle:false,requestedEnabled:true,enabled:true},
   {id:'selected',name:'Selected',canToggle:true,requestedEnabled:true,enabled:true},
   {id:'off',name:'Off',canToggle:true,requestedEnabled:false,enabled:false},
   {id:'failed',name:'Failed',canToggle:true,requestedEnabled:true,enabled:false,error:'process exited'}
  ].map(p=>({description:'',origin:'builtin',hasUi:false,error:null,...p}));
  const roster=()=>plugins.map(p=>({...p,optionalEnabled}));
  window.calls=[];window.fail=false;window.hold=false;
  configureClient(createClient(async(method,params)=>{
   if(method==='list_plugins')return roster();
   window.calls.push({method,params});
   if(window.hold)await new Promise(resolve=>{window.release=resolve;});
   if(window.fail)throw Error('offline');
   if(method==='set_optional_plugins_enabled')optionalEnabled=params.enabled;
   else if(method==='set_plugin_enabled')plugins=plugins.map(p=>p.id===params.id?{...p,requestedEnabled:params.enabled}:p);
   else throw Error(method);
   plugins=plugins.map(p=>({...p,enabled:!p.canToggle || (optionalEnabled && p.requestedEnabled && !p.error)}));
   return roster();
  }));
  createRoot(document.getElementById('root')).render(<Plugins/>);
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 await page.route('http://master.test/**',route=>route.fulfill({contentType:route.request().url().endsWith('/app.js')?'text/javascript':'text/html',body:route.request().url().endsWith('/app.js')?result.outputFiles[0].text:'<div id="root"></div><script type="module" src="/app.js"></script>'}));
 await page.goto('http://master.test/');
 const master=page.getByRole('switch',{name:'Несистемные плагины'});
 const row=name=>page.locator('.plugin-row').filter({has:page.locator('.plugin-name',{hasText:name})});
 await expect(master).toBeChecked();
 await page.evaluate(()=>{window.hold=true;});await master.click();
 await expect(master).toBeDisabled();await expect(master).toHaveAttribute('aria-busy','true');
 await page.evaluate(()=>{window.hold=false;window.release();});
 await expect(master).not.toBeChecked();
 await expect(row('Manager').locator('.plugin-status')).toHaveText('Работает');
 await expect(row('Selected').locator('.plugin-status')).toHaveText('Приостановлен');
 await expect(row('Failed').getByRole('alert')).toHaveText('process exited');
 await row('Selected').getByRole('switch').click();
 await expect(row('Selected').locator('.plugin-status')).toHaveText('Отключён');
 await row('Off').getByRole('switch').click();
 await expect(row('Off').locator('.plugin-status')).toHaveText('Приостановлен');
 await master.click();
 await expect(row('Selected').locator('.plugin-status')).toHaveText('Отключён');
 await expect(row('Off').locator('.plugin-status')).toHaveText('Работает');
 expect(await page.evaluate(()=>window.calls)).toEqual([
  {method:'set_optional_plugins_enabled',params:{enabled:false}},
  {method:'set_plugin_enabled',params:{id:'selected',enabled:false}},
  {method:'set_plugin_enabled',params:{id:'off',enabled:true}},
  {method:'set_optional_plugins_enabled',params:{enabled:true}},
 ]);
 await page.evaluate(()=>{window.fail=true;});await master.click();
 await expect(page.getByRole('alert').filter({hasText:'offline'})).toBeVisible();
 await expect(master).toBeChecked();await expect(master).toBeEnabled();
});
