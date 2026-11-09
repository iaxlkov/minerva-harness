import {viteWorkerStub} from './vite-worker-stub.mjs';
import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import {readFile} from 'node:fs/promises';

test('external React settings reuse module and draft across panel remounts',async({page})=>{
  const source=`
    const React=require('react'),{Button}=require('@minerva/ui');
    window.moduleEvaluations=(window.moduleEvaluations||0)+1;
    exports.sdkVersion=2;
    exports.apply=ctx=>{
      let draft='',saved='';const listeners=new Set();
      const subscribe=fn=>{listeners.add(fn);return ()=>listeners.delete(fn);};
      const changed=()=>{for(const fn of listeners)fn();};
      ctx.effect(()=>{window.pluginDisposed=(window.pluginDisposed||0)+1;listeners.clear();});
      function Body(){
        React.useSyncExternalStore(subscribe,()=>draft+'|'+saved);
        return React.createElement('section',null,
          React.createElement('h2',null,'Fixture settings'),
          React.createElement('input',{'aria-label':'Draft',value:draft,onChange:e=>{draft=e.target.value;changed();}}),
          React.createElement(Button,{onClick:async()=>{saved=await ctx.rpc('settings.save',{draft});changed();}},'Save'),
          React.createElement('p',{role:'status'},saved));
      }
      ctx.slots.register('settings.pages',{entryId:'plugin:fixture',component:props=>props.placement==='navigation'
        ?React.createElement('button',{className:props.selectedPage==='plugin:fixture'?'selected':'','aria-current':props.selectedPage==='plugin:fixture'?'page':undefined,onClick:()=>props.onSelectPage('plugin:fixture')},'Fixture')
        :React.createElement(Body)});
    };
  `;
  const result=await build({plugins:[viteWorkerStub],stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
    import React from 'react';
    import {createRoot} from 'react-dom/client';
    import {NavigationShell} from './apps/web/src/app/NavigationShell';
 const Settings=()=> <NavigationShell session={{id:'fixture',title:'Fixture',model:null,updatedAt:0}} sessions={[]} onSession={()=>{}} onNew={()=>{}} providers={[]} onProviders={()=>{}} activePlugins={[]}><p>Conversation</p></NavigationShell>;
    import {PluginRoot,createBrowserComposition} from './apps/web/src/app/plugins';
    import {startExternalPlugins} from './apps/web/src/app/externalPlugins';
    import {models} from './apps/web/src/app/models';
    import {configureClient,createClient} from './packages/client/src/index';
    const plugin={id:'fixture',name:'Fixture',enabled:true,hasUi:true,origin:'installed',canToggle:true,description:''};
    models.plugins.set([plugin,{...plugin,id:'disabled',name:'Disabled',enabled:false},{...plugin,id:'headless',name:'Headless',hasUi:false}]);
    window.setPluginEnabled=enabled=>models.plugins.set([{...plugin,enabled}]);
    window.calls=[];
    configureClient(createClient(async(method,params)=>{window.calls.push({method,params});if(method==='workspaces.list')return [];if(method==='plugins.ui')return ${JSON.stringify(source)};if(method==='plugins.call')return 'Saved';throw Error(method);}));
    createBrowserComposition().then(host=>{const externalPlugins=startExternalPlugins(host);createRoot(document.getElementById('root')).render(<PluginRoot host={host} externalPlugins={externalPlugins}><Settings/></PluginRoot>);});
  `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
  const css=await readFile('apps/web/src/style.css','utf8')+'\n'+await readFile('apps/web/src/app/navigation-shell.css','utf8');
  const csp="default-src 'self'; script-src 'self' blob:; style-src 'self' 'unsafe-inline'; object-src 'none'";
  await page.route('http://settings.test/**',route=>{
    const script=route.request().url().endsWith('/app.js');
    return route.fulfill({headers:{'Content-Security-Policy':csp},contentType:script?'text/javascript':'text/html',body:script?result.outputFiles[0].text:`<style>${css}</style><div id="root"></div><script type="module" src="/app.js"></script>`});
  });
  await page.goto('http://settings.test/');
  await page.locator('.settings-open').click();
  const dialog=page.locator('.navigation-shell');
  await expect(dialog.getByRole('button',{name:'Disabled',exact:true})).toHaveCount(0);
  await expect(dialog.getByRole('button',{name:'Headless',exact:true})).toHaveCount(0);
  await dialog.getByRole('button',{name:'Fixture',exact:true}).click();
  await expect(dialog.getByRole('heading',{name:'Fixture settings'})).toBeVisible();
  await expect(page.locator('iframe')).toHaveCount(0);
  await dialog.getByRole('textbox',{name:'Draft'}).fill('unsaved preference');
  await dialog.getByRole('button',{name:'Модели',exact:true}).click();
  await dialog.getByRole('button',{name:'Fixture',exact:true}).click();
  await expect(dialog.getByRole('textbox',{name:'Draft'})).toHaveValue('unsaved preference');
  await page.getByRole('button',{name:'Чаты',exact:true}).click();
  await expect(page.locator('.settings-screen')).toHaveCount(0);
  await page.locator('.settings-open').click();
  await expect(dialog.getByRole('textbox',{name:'Draft'})).toHaveValue('unsaved preference');
  expect(await page.evaluate(()=>window.moduleEvaluations)).toBe(1);
  expect(await page.evaluate(()=>window.calls.filter(c=>c.method==='plugins.ui').length)).toBe(1);
  await dialog.getByRole('button',{name:'Save',exact:true}).click();
  await expect(dialog.getByRole('status')).toHaveText('Saved');
  expect(await page.evaluate(()=>window.calls)).toContainEqual({method:'plugins.call',params:{id:'fixture',method:'settings.save',params:{draft:'unsaved preference'}}});
  await page.evaluate(()=>window.setPluginEnabled(false));
  await expect(dialog.getByRole('button',{name:'Fixture',exact:true})).toHaveCount(0);
  await expect(dialog.getByRole('button',{name:'Модели',exact:true})).toHaveAttribute('aria-current','page');
  expect(await page.evaluate(()=>window.pluginDisposed)).toBe(1);
  await page.evaluate(()=>window.setPluginEnabled(true));
  await dialog.getByRole('button',{name:'Fixture',exact:true}).click();
  await expect(dialog.getByRole('textbox',{name:'Draft'})).toHaveValue('');
  expect(await page.evaluate(()=>window.moduleEvaluations)).toBe(2);
});
