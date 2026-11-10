import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import postcss from 'postcss';
import tailwind from 'tailwindcss';
import {readFile} from 'node:fs/promises';

for(const width of [1280,360])test(`plugin error recovery at ${width}px`,async({page},testInfo)=>{
 const bundle=await build({stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
  import React from 'react';import {createRoot} from 'react-dom/client';
  import {BrowserPluginHost} from './packages/plugin-sdk/src/index';
  import {SlotRenderer} from './packages/plugin-sdk/src/renderer';
  import {PluginError} from './apps/web/src/app/PluginError';
  const host=new BrowserPluginHost();
  await host.activate([{id:'broken-plugin',apply(ctx){ctx.slots.register('root.layout',{entryId:'broken',component:()=>{throw Error('Cannot render view: missing component');}});}}]);
  let attempts=0;window.settingsOpened=0;
  createRoot(document.getElementById('root')).render(<SlotRenderer registry={host.slots} slot="root.layout" scope={{kind:'root'}} fallback={<p>Recovered</p>} onOpenSettings={()=>{window.settingsOpened++;}} onDisable={async id=>{if(++attempts===1)throw Error('Server unavailable');await host.disable(id);}} renderError={props=><PluginError {...props} fullPage/>}/>);
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 const css=(await postcss([tailwind]).process(await readFile('apps/web/src/style.css','utf8'),{from:'apps/web/src/style.css'})).css+'\n'+await readFile('apps/web/src/app/navigation-shell.css','utf8')+'\n'+await readFile('apps/web/src/app/plugin-error.css','utf8');
 await page.setViewportSize({width,height:800});
 await page.route('http://plugin-error.test/**',route=>route.fulfill({contentType:route.request().url().endsWith('.js')?'text/javascript':'text/html',body:route.request().url().endsWith('.js')?bundle.outputFiles[0].text:`<style>${css}</style><div id="root"></div><script type="module" src="/app.js"></script>`}));
 await page.goto('http://plugin-error.test');
 await expect(page.getByRole('alert')).toContainText('Cannot render view: missing component');
 const content=page.locator('.plugin-error-content');
 const box=await content.boundingBox();
 expect(Math.abs(box.x+box.width/2-width/2)).toBeLessThan(2);
 expect(Math.abs(box.y+box.height/2-400)).toBeLessThan(2);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBe(width);
 const buttons=page.locator('[data-slot="button"]');
 await expect(buttons).toHaveCount(2);
 await buttons.first().click();
 expect(await page.evaluate(()=>window.settingsOpened)).toBe(1);
 await page.screenshot({path:testInfo.outputPath('plugin-error.png')});
 await buttons.last().click();
 await expect(page.getByRole('alert')).toContainText('Server unavailable');
 await expect(page.getByRole('alert')).toContainText('Cannot render view: missing component');
 await buttons.last().click();
 await expect(page.getByText('Recovered',{exact:true})).toBeVisible();
});
