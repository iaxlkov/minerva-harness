import {viteWorkerStub} from './vite-worker-stub.mjs';
import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import postcss from 'postcss';
import tailwind from 'tailwindcss';
import {readFile} from 'node:fs/promises';

export async function mount(page, {withMessages=false}={}) {
 const bundle=await build({plugins:[viteWorkerStub],stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
  import React from 'react';import {createRoot} from 'react-dom/client';
  import {DropdownMenu,DropdownMenuTrigger,DropdownMenuContent,DropdownMenuItem} from './apps/web/src/components/ui/dropdown-menu';
  import {Button} from './apps/web/src/components/ui/button';
  import {Input} from './apps/web/src/components/ui/input';
  import {Chat} from './apps/web/src/pages/chat/Chat';
  import {configureClient} from './apps/web/src/client';
  import {PluginRoot,createBrowserComposition} from './apps/web/src/app/plugins';
  configureClient({call:async(method)=>{
   if(method==='session_load'&&${withMessages})return [
    {id:'question',parentId:null,role:'user',createdAt:'2026-01-01T00:00:00Z',extensions:{},parts:[{kind:'text',data:{text:'Что умеет Минерва?'}}]},
    {id:'answer',parentId:'question',role:'assistant',createdAt:'2026-01-01T00:00:01Z',extensions:{},parts:[{kind:'text',data:{text:'Минерва помогает работать с файлами проекта, проверять изменения и находить нужный код. '.repeat(24)}}]},
   ];
   if(method==='workspaces.list')return [{id:'default',title:'Default workspace',path:null,missing:false,sessionIds:Array.from({length:40},(_,index)=>String(index))}];
   if(method==='context.status')return {estimatedTokens:34,contextWindow:32768,availableTokens:32734,usage:{inputTokens:34,outputTokens:12,totalTokens:46}};
   if(method==='workspace.get')return {cwd:null,permissionPolicy:'ask',contextWindow:32768,autoCompact:true};
   if(method==='shell_settings')return {timeoutMs:30000,maxOutputBytes:10000};
   if(method==='list_plugins')return [{id:'shell',name:'Командная оболочка',description:'Выполнение команд',origin:'builtin',enabled:true}];
   return [];
  }});
  const session={id:'layout',title:'Новый диалог',model:null,updatedAt:0};
  createBrowserComposition().then(host=>{window.disableSection=()=>host.disable('shell-fixture');window.enableSection=()=>host.activate([{id:'shell-fixture',apply(ctx){
   const scoped=ctx.scope({kind:'workspace',workspaceId:'default'});
   scoped.slots.register('shell.sections',{entryId:'fixture:reports',component:props=>props.placement==='title'?'Отчёты':props.placement==='rail'?<DropdownMenu><DropdownMenuTrigger className={'rail-button '+(props.selected?'selected':'')} aria-label="Отчёты">▦</DropdownMenuTrigger><DropdownMenuContent side="right"><DropdownMenuItem onSelect={()=>props.onNavigate('fixture:reports')}>Открыть отчёты</DropdownMenuItem></DropdownMenuContent></DropdownMenu>:props.placement==='sidebar'?<Button variant="ghost" className="nav-item">Мои отчёты</Button>:<section style={{padding:32}}><h1>Отчёты проекта</h1><Input aria-label="Фильтр отчётов"/></section>});
   for(const name of ['shell.rail.start','shell.rail.end','shell.sidebar.header','shell.sidebar.content','shell.sidebar.footer','shell.header.actions','shell.titlebar.start','shell.titlebar.end','shell.content.before','shell.content.after'])scoped.slots.register(name,{entryId:'marker',component:()=> <span data-testid={name}>·</span>});
  }}]);createRoot(document.getElementById('root')).render(<PluginRoot host={host}><Chat session={session} sessions={Array.from({length:40},(_,index)=>({...session,id:String(index),title:"Session "+index}))} providers={[]} pluginModels={[]} model="" configured={false} activePlugins={[]} onSelect={()=>{}} onProviders={()=>{}} onNew={()=>{}} onSession={()=>{}} onSaved={()=>{}} onError={()=>{}}/></PluginRoot>);});
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 const css=(await postcss([tailwind]).process(await readFile('apps/web/src/style.css','utf8'),{from:'apps/web/src/style.css'})).css+'\n'+await readFile('apps/web/src/harness/harness.css','utf8')+'\n'+await readFile('apps/web/src/app/navigation-shell.css','utf8')+'\n'+await readFile('apps/web/src/app/title-bar.css','utf8')+'\n'+await readFile('apps/web/src/app/canvas.css','utf8');
 await page.route('http://layout.test/**',async route=>{
  const url=new URL(route.request().url());
  if(url.pathname==='/minerva-welcome.png'||url.pathname==='/minerva-64.png')return route.fulfill({contentType:'image/png',body:await readFile('apps/web/public'+url.pathname)});
  if(url.pathname.startsWith('/api/'))return route.abort();
  return route.fulfill({contentType:url.pathname==='/app.js'?'text/javascript':'text/html',body:url.pathname==='/app.js'?bundle.outputFiles[0].text:`<style>${css}</style><div id="root"></div><script type="module" src="/app.js"></script>`});
 });
 await page.goto('http://layout.test');
}

for(const [width,height] of [[1280,800],[900,800],[700,800],[900,550]])test(`screen layout at ${width}x${height}`,async({page},testInfo)=>{
 await page.setViewportSize({width,height});await mount(page,{withMessages:true});
 await expect(page.getByText('Что умеет Минерва?')).toBeVisible();
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
 const composer=await page.locator('[data-slot="composer-bar"]').boundingBox();
 const messages=await page.locator('.messages').boundingBox();
 expect(Math.abs(composer.x-messages.x)).toBeLessThanOrEqual(1);
 expect(Math.abs(composer.width-messages.width)).toBeLessThanOrEqual(1);
 expect(composer.y+composer.height).toBeLessThanOrEqual(height);
 const bar=page.locator('[data-slot="composer-bar"]');
 if(width<=900){
  const input=await bar.locator('textarea').boundingBox();
  const controls=await bar.locator('.composer-access-actions').boundingBox();
  expect(input.width).toBeGreaterThan(composer.width*.8);
  expect(input.y+input.height).toBeLessThanOrEqual(controls.y);
 }
 const border=()=>bar.evaluate(el=>getComputedStyle(el).borderColor);
 const initialBorder=await border();
 await bar.locator('textarea').focus();
 expect(await border()).toBe(initialBorder);
 expect(await bar.evaluate(el=>getComputedStyle(el).borderRadius)).toBe('24px');
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
 const checkbox=page.getByRole('switch',{name:'Автоматически сжимать контекст'});
 await expect(checkbox).toBeVisible();
 expect((await checkbox.boundingBox()).width).toBeLessThan(40);
 await checkOverflow('.settings-body');
 await page.screenshot({path:testInfo.outputPath('workspace.png')});
 await page.getByRole('button',{name:'Серверы MCP',exact:true}).click();
 await page.getByRole('button',{name:'Добавить сервер MCP'}).click();
 await expect(page.getByLabel('ID сервера')).toBeVisible();
 await checkOverflow('.settings-body');
 await page.screenshot({path:testInfo.outputPath('mcp.png')});
 await page.getByRole('button',{name:'Чаты',exact:true}).click();
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


test('shell keeps draft, resizes, persists preferences and removes plugin sections',async({page},testInfo)=>{
 await page.setViewportSize({width:1280,height:800});await mount(page);
 const input=page.getByRole('textbox',{name:'Сообщение ассистенту'});
 await input.fill('Черновик для проверки переходов');
 const sidebar=page.locator('#section-sidebar'),resize=page.getByRole('separator',{name:'Ширина боковой панели'});
 const initial=(await sidebar.boundingBox()).width,handle=await resize.boundingBox();
 expect(handle.width).toBe(1);
 await page.mouse.move(handle.x-4,200);await page.mouse.down();await page.mouse.move(handle.x+96,200,{steps:40});
 expect((await sidebar.boundingBox()).width).toBeCloseTo(initial+100,0);
 await page.mouse.up();
 expect((await sidebar.boundingBox()).width).toBeCloseTo(initial+100,0);
 const expandedHandle=await resize.boundingBox();
 await page.mouse.move(expandedHandle.x+4,200);await page.mouse.down();await page.mouse.move(expandedHandle.x-56,200,{steps:40});
 expect((await sidebar.boundingBox()).width).toBeCloseTo(initial+40,0);
 await page.mouse.up();
 expect((await sidebar.boundingBox()).width).toBeCloseTo(initial+40,0);
 await resize.focus();await page.keyboard.press('ArrowLeft');const savedWidth=(await sidebar.boundingBox()).width;expect(savedWidth).toBeLessThan(initial+100);
 await page.getByRole('button',{name:'Боковая панель',exact:true}).click();await expect(sidebar).toBeHidden();
 await page.getByRole('button',{name:'Плагины',exact:true}).click();await expect(page.locator('.plugins-screen')).toBeVisible();
 await page.getByRole('button',{name:'Боковая панель',exact:true}).click();
 await expect(page.locator('.workspace-list')).toHaveCount(0);await expect(sidebar.getByText('Мои плагины')).toBeVisible();
 await page.getByRole('button',{name:'Настройки',exact:true}).click();await expect(page.getByRole('dialog')).toHaveCount(0);await expect(sidebar.getByRole('button',{name:'Модели',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Чаты',exact:true}).click();await expect(input).toHaveValue('Черновик для проверки переходов');
 await page.evaluate(()=>window.enableSection());await page.getByRole('button',{name:'Отчёты',exact:true}).click();await page.getByRole('menuitem',{name:'Открыть отчёты'}).click();
 await expect(sidebar.getByRole('button',{name:'Мои отчёты'})).toBeVisible();await expect(page.getByRole('heading',{name:'Отчёты проекта'})).toBeVisible();
 for(const name of ['shell.rail.start','shell.rail.end','shell.sidebar.header','shell.sidebar.content','shell.sidebar.footer','shell.header.actions','shell.titlebar.start','shell.titlebar.end','shell.content.before','shell.content.after'])await expect(page.getByTestId(name)).toBeVisible();
 await page.screenshot({path:testInfo.outputPath('plugin-section.png')});
 await page.evaluate(()=>window.disableSection());await expect(page.getByRole('button',{name:'Отчёты',exact:true})).toHaveCount(0);await expect(input).toHaveValue('Черновик для проверки переходов');
 await page.getByRole('button',{name:'Боковая панель',exact:true}).click();await page.reload();await expect(sidebar).toBeHidden();
 await page.getByRole('button',{name:'Боковая панель',exact:true}).click();expect((await sidebar.boundingBox()).width).toBeCloseTo(savedWidth,0);
 await page.getByRole('separator',{name:'Ширина боковой панели'}).dblclick();expect((await sidebar.boundingBox()).width).toBeCloseTo(272,0);
});

test('narrow screen opens sidebar over content and closes it on navigation',async({page},testInfo)=>{
 await page.setViewportSize({width:390,height:844});await mount(page);
 await expect(page.locator('#section-sidebar')).toBeHidden();
 await page.getByRole('button',{name:'Боковая панель',exact:true}).click();await expect(page.locator('#section-sidebar')).toBeVisible();
 await page.getByRole('button',{name:'Закрыть боковую панель'}).click();await expect(page.locator('#section-sidebar')).toBeHidden();
 await page.getByRole('button',{name:'Настройки',exact:true}).click();await expect(page.locator('.settings-screen')).toBeVisible();await expect(page.getByRole('button',{name:'Настройки',exact:true})).toHaveAttribute('aria-current','page');
 await page.getByRole('button',{name:'Боковая панель',exact:true}).click();
 await page.getByRole('button',{name:'Серверы MCP',exact:true}).click();await expect(page.getByRole('dialog')).toHaveCount(0);
 await expect(page.getByRole('button',{name:'Добавить сервер MCP'})).toBeVisible();
 await page.screenshot({path:testInfo.outputPath('mobile-settings.png')});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
});

test('empty conversation sits in the vertical centre of the thread',async({page})=>{
 await page.setViewportSize({width:1280,height:900});await mount(page);
 const thread=await page.locator('.thread').boundingBox(),view=await page.locator('.thread .viewport').boundingBox(),composer=await page.locator('.composer-wrap').boundingBox();
 expect(Math.abs((view.y+composer.y+composer.height)/2-(thread.y+thread.height/2))).toBeLessThan(4);
});
