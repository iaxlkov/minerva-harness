import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import {viteWorkerStub} from '../../../../../tests/e2e/vite-worker-stub.mjs';

test('memory readback and disabled panel',async({page})=>{
 const result=await build({plugins:[viteWorkerStub],stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
 import React from 'react'; import {createRoot} from 'react-dom/client';
 import {Plugins} from './apps/web/src/pages/plugins/Plugins'; import {configureClient,createClient} from './packages/client/src/index';
 let memory=[]; let enabled=true; window.offline=false;
 configureClient(createClient(async(method,params)=>{
   if(method==='list_plugins') return [{id:'memory',name:'Memory',description:'',origin:'builtin-optional',hasUi:false,canToggle:true,requestedEnabled:enabled,enabled,optionalEnabled:true,error:null}];
   if(method==='model_providers')return [];
   if(method==='session_list') return [{id:'s',title:'Chat',updatedAt:0,model:null}];
   if(method==='set_plugin_enabled'){enabled=params.enabled;return [{id:'memory',name:'Memory',description:'',origin:'builtin-optional',hasUi:false,canToggle:true,requestedEnabled:enabled,enabled,optionalEnabled:true,error:null}];}
   if(method==='plugins.call') { if(window.offline) throw Error('offline'); const {method:m,params:p}=params; if(m==='remember'){memory=[{id:'one',text:p.payload.text}];return {};} if(m==='search')return memory; if(m==='read')return memory[0]; }
   throw Error('unexpected '+method);
 })); createRoot(document.getElementById('root')).render(<Plugins/>);
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 await page.route('https://learning.test/**',route=>route.fulfill({contentType:route.request().url().endsWith('/app.js')?'text/javascript':'text/html',body:route.request().url().endsWith('/app.js')?result.outputFiles[0].text:'<div id="root"></div><script type="module" src="/app.js"></script>'}));
 await page.goto('https://learning.test/'); await page.getByRole('button',{name:'Открыть'}).click();
 await page.getByLabel('Содержание').fill('Важное правило'); await page.getByRole('button',{name:'Сохранить'}).click();
 await expect(page.locator('.plugin-row').getByText('Важное правило')).toBeVisible(); await page.getByRole('button',{name:'Открыть'}).click(); await expect(page.getByText('text:')).toBeVisible();
 await page.evaluate(()=>window.offline=true); await page.getByRole('button',{name:'Обновить'}).click(); await expect(page.getByRole('alert')).toContainText('offline');
 await page.getByRole('button',{name:'Назад'}).click(); await page.getByRole('button',{name:'Отключить',exact:true}).click(); await page.getByRole('button',{name:'Открыть'}).click(); await expect(page.getByText('Включите плагин в списке', {exact:false})).toBeVisible();
});
