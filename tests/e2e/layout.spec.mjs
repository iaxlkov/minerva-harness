import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import postcss from 'postcss';
import tailwind from 'tailwindcss';
import {readFile} from 'node:fs/promises';

async function mount(page) {
 const bundle=await build({stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
  import React from 'react';import {createRoot} from 'react-dom/client';
  import {Chat} from './apps/web/src/pages/chat/Chat';
  import {configureClient} from './apps/web/src/client';
  import {PluginRoot,createBrowserComposition} from './apps/web/src/app/plugins';
  configureClient({call:async(method)=>{
   if(method==='workspaces.list')return [{id:'default',title:'Default workspace',path:null,missing:false,sessionIds:Array.from({length:40},(_,index)=>String(index))}];
   if(method==='context.status')return {estimatedTokens:34,contextWindow:32768,availableTokens:32734};
   if(method==='workspace.get')return {cwd:null,permissionPolicy:'ask',contextWindow:32768,autoCompact:true};
   if(method==='shell_settings')return {timeoutMs:30000,maxOutputBytes:10000};
   if(method==='list_plugins')return [{id:'shell',name:'Командная оболочка',description:'Выполнение команд',origin:'builtin',enabled:true}];
   return [];
  }});
  const session={id:'layout',title:'Новый диалог',model:null,updatedAt:0};
  createBrowserComposition().then(host=>createRoot(document.getElementById('root')).render(<PluginRoot host={host}><Chat session={session} sessions={Array.from({length:40},(_,index)=>({...session,id:String(index),title:"Session "+index}))} providers={[]} pluginModels={[]} model="" configured={false} activePlugins={[]} onSelect={()=>{}} onProviders={()=>{}} onNew={()=>{}} onSession={()=>{}} onSaved={()=>{}} onError={()=>{}}/></PluginRoot>));
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 const css=(await postcss([tailwind]).process(await readFile('apps/web/src/style.css','utf8'),{from:'apps/web/src/style.css'})).css+'\n'+await readFile('apps/web/src/harness/harness.css','utf8');
 await page.route('http://layout.test/**',async route=>{
  const url=new URL(route.request().url());
  if(url.pathname==='/minerva-welcome.png'||url.pathname==='/minerva-64.png')return route.fulfill({contentType:'image/png',body:await readFile('apps/web/public'+url.pathname)});
  if(url.pathname.startsWith('/api/'))return route.abort();
  return route.fulfill({contentType:url.pathname==='/app.js'?'text/javascript':'text/html',body:url.pathname==='/app.js'?bundle.outputFiles[0].text:`<style>${css}</style><div id="root"></div><script type="module" src="/app.js"></script>`});
 });
 await page.goto('http://layout.test');
}

for(const [width,height] of [[1280,800],[900,800],[700,800],[900,550]])test(`screen layout at ${width}x${height}`,async({page},testInfo)=>{
 await page.setViewportSize({width,height});await mount(page);
 await page.locator('.brand').screenshot({path:testInfo.outputPath('brand.png')});
 const checkOverflow=async selector=>{
  expect(await page.locator(selector).evaluate(element=>element.scrollWidth-element.clientWidth)).toBeLessThanOrEqual(1);
 };
 const sessions=page.locator('.workspace-list');
 await expect(sessions).toHaveAttribute('data-slot','scroll-area');
 expect(await sessions.evaluate(el=>getComputedStyle(el).scrollbarWidth)).toBe('thin');
 expect(await sessions.evaluate(el=>el.scrollHeight>el.clientHeight)).toBe(true);
 await sessions.getByRole('button').last().focus();
 await expect.poll(()=>sessions.evaluate(el=>el.scrollTop)).toBeGreaterThan(0);
 const budget=await page.locator('.harness-session').boundingBox(),thread=await page.locator('.thread').boundingBox();
 expect(thread.y).toBeGreaterThanOrEqual(budget.y+budget.height-1);
 expect(Math.abs(thread.x-budget.x)).toBeLessThan(1);
 expect(thread.width).toBeCloseTo(budget.width,0);
 const composer=await page.locator('.composer').boundingBox();
 expect(composer.y+composer.height).toBeLessThanOrEqual(height);
 await page.getByRole('button',{name:'Добавить вложение'}).click();
 const menu=page.getByRole('menu');
 await expect(menu).toBeVisible();
 const menuBox=await menu.boundingBox();
 expect(Math.abs(menuBox.x-composer.x)).toBeLessThanOrEqual(2);
 expect(Math.abs(menuBox.width-composer.width)).toBeLessThanOrEqual(2);
 expect(menuBox.y+menuBox.height).toBeLessThan(composer.y);
 await expect(menu.getByText('Добавить',{exact:true})).toHaveCount(0);
 await page.screenshot({path:testInfo.outputPath('composer-menu.png')});
 await page.keyboard.press('Escape');
 await checkOverflow('.main');
 await page.screenshot({path:testInfo.outputPath('chat.png')});
 await page.getByRole('button',{name:'Настройки',exact:true}).click();
 await page.getByRole('button',{name:'Добавить провайдера моделей'}).click();
 await expect(page.getByLabel('ID провайдера',{exact:true})).toBeVisible();
 await checkOverflow('.settings-body');
 await page.screenshot({path:testInfo.outputPath('models.png')});
 await page.getByRole('button',{name:'Сессия',exact:true}).click();
 const checkbox=page.getByRole('checkbox',{name:'Автоматически сжимать контекст'});
 await expect(checkbox).toBeVisible();
 expect((await checkbox.boundingBox()).width).toBeLessThan(24);
 await checkOverflow('.settings-body');
 await page.screenshot({path:testInfo.outputPath('workspace.png')});
 await page.getByRole('button',{name:'Серверы MCP',exact:true}).click();
 await page.getByRole('button',{name:'Добавить сервер MCP'}).click();
 await expect(page.getByLabel('ID сервера')).toBeVisible();
 await checkOverflow('.settings-body');
 await page.screenshot({path:testInfo.outputPath('mcp.png')});
 await page.getByRole('button',{name:'Закрыть настройки'}).click();
 await page.getByRole('button',{name:'Плагины',exact:true}).click();
 await expect(page.getByRole('button',{name:'Настроить',exact:true})).toBeVisible();
 await checkOverflow('.plugins-screen');
 await page.screenshot({path:testInfo.outputPath('plugins.png')});
 await page.getByRole('button',{name:'Настроить',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Командная оболочка'})).toBeVisible();
 await checkOverflow('.shell-screen');
 await page.screenshot({path:testInfo.outputPath('shell.png')});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
});
