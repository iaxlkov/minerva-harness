import {test,expect} from '@playwright/test';
import {build} from 'esbuild';

test('polled crash removes external settings and exposes retry and disable actions',async({page})=>{
 const source=`exports.sdkVersion=2;exports.apply=ctx=>{const React=require('react');ctx.effect(()=>{window.disposed=true;});ctx.slots.register('settings.pages',{entryId:'plugin:fixture',component:props=>props.placement==='navigation'?React.createElement('button',{onClick:()=>props.onSelectPage('plugin:fixture')},'Fixture'):React.createElement('p',null,'Fixture body')});};`;
 const result=await build({stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
  import React from 'react';
  import {createRoot} from 'react-dom/client';
  import {Plugins} from './apps/web/src/pages/plugins/Plugins';
  import {NavigationShell} from './apps/web/src/app/NavigationShell';
 const Settings=()=> <NavigationShell session={{id:'fixture',title:'Fixture',model:null,updatedAt:0}} sessions={[]} onSession={()=>{}} onNew={()=>{}} providers={[]} onProviders={()=>{}} activePlugins={[]}><p>Conversation</p></NavigationShell>;
  import {PluginRoot,createBrowserComposition} from './apps/web/src/app/plugins';
  import {startExternalPlugins} from './apps/web/src/app/externalPlugins';
  import {models} from './apps/web/src/app/models';
  import {configureClient,createClient} from './packages/client/src/index';
  let plugin={id:'fixture',name:'Fixture',description:'Fixture plugin',enabled:true,requestedEnabled:true,optionalEnabled:true,error:null,canToggle:true,origin:'installed',hasUi:true};
  window.crash=()=>{plugin={...plugin,enabled:false,error:'process exited'};};
  window.failTransport=false;window.toggles=[];
  configureClient(createClient(async(method,params)=>{
   if(method==='list_plugins'){if(window.failTransport)throw Error('offline');return [plugin];}
   if(method==='set_plugin_enabled'){window.toggles.push(params);plugin={...plugin,enabled:params.enabled,requestedEnabled:params.enabled,error:params.enabled?null:plugin.error};return [plugin];}
   if(method==='workspaces.list')return [];if(method==='plugins.ui')return ${JSON.stringify(source)};
   throw Error(method);
  }));
  createBrowserComposition().then(host=>{
   const externalPlugins=startExternalPlugins(host);models.plugins.startPolling();
   createRoot(document.getElementById('root')).render(<PluginRoot host={host} externalPlugins={externalPlugins}><Settings/></PluginRoot>);
  });
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 await page.route('http://roster.test/**',route=>route.fulfill({contentType:route.request().url().endsWith('/app.js')?'text/javascript':'text/html',body:route.request().url().endsWith('/app.js')?result.outputFiles[0].text:'<div id="root"></div><script type="module" src="/app.js"></script>'}));
 await page.goto('http://roster.test/');
 await page.locator('.settings-open').click();
 const dialog=page.locator('.navigation-shell');
 await expect(dialog.getByRole('button',{name:'Fixture',exact:true})).toBeVisible();
 await page.evaluate(()=>window.crash());
 await expect(dialog.getByRole('button',{name:'Fixture',exact:true})).toHaveCount(0);
 expect(await page.evaluate(()=>window.disposed)).toBe(true);
 await page.getByRole('button',{name:'Плагины',exact:true}).click();
 await expect(page.getByRole('alert')).toHaveText('process exited');
 await page.getByRole('button',{name:'Отключить автозапуск'}).click();
 expect(await page.evaluate(()=>window.toggles.at(-1))).toEqual({id:'fixture',enabled:false});
 await expect(page.getByRole('alert')).toHaveText('process exited');
 await page.getByRole('button',{name:'Повторить запуск'}).click();
 await expect(page.getByRole('alert')).toHaveCount(0);
 await expect(page.locator('.plugin-status')).toHaveText('Работает');
 await page.evaluate(()=>{window.failTransport=true;});
 await expect(page.getByRole('alert')).toContainText('offline');
 await expect(page.locator('.plugin-status')).toHaveText('Работает');
});
