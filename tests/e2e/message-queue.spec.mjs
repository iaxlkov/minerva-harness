import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import {viteWorkerStub} from './vite-worker-stub.mjs';

test('queue keeps files, sends FIFO and removes pending messages',async({page},info)=>{
 const bundle=await build({plugins:[viteWorkerStub],stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
 import React from 'react';import {createRoot} from 'react-dom/client';
 import {AssistantRuntimeProvider,useLocalRuntime,ComposerPrimitive} from '@assistant-ui/react';
 import {PluginRoot,createBrowserComposition,UiSlot,workspaceScope} from './apps/web/src/app/plugins';
 import {messageQueueOptions} from './apps/web/src/plugins/message-queue';
 window.sent=[];window.finish=()=>{};
 function App(){const runtime=useLocalRuntime({async run({messages}){window.sent.push(messages.at(-1));await new Promise(resolve=>window.finish=resolve);return {content:[{type:'text',text:'Done'}]};}}, {...messageQueueOptions,adapters:{attachments:{accept:'*',async add({file}){return {id:file.name,type:'file',name:file.name,file,contentType:file.type,status:{type:'requires-action',reason:'composer-send'}};},async send(a){return {...a,status:{type:'complete'},content:[{type:'file',data:'proof',mimeType:'text/plain',filename:a.name}]};},async remove(){}}}});return <AssistantRuntimeProvider runtime={runtime}><ComposerPrimitive.Root><UiSlot slot="composer.queue" scope={workspaceScope} componentProps={{}}/><ComposerPrimitive.Input aria-label="Message"/><ComposerPrimitive.AddAttachment>Attach</ComposerPrimitive.AddAttachment><ComposerPrimitive.Send>Send</ComposerPrimitive.Send></ComposerPrimitive.Root></AssistantRuntimeProvider>}
 createBrowserComposition().then(host=>createRoot(document.getElementById('root')).render(<PluginRoot host={host}><App/></PluginRoot>));
 `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
 await page.route('http://queue.test/**',route=>route.fulfill({contentType:'text/html',body:`<div id="root"></div><script type="module">${bundle.outputFiles[0].text}</script>`}));
 await page.goto('http://queue.test');
 await page.getByLabel('Message').fill('First');await page.getByText('Send',{exact:true}).click();
 await expect.poll(()=>page.evaluate(()=>window.sent.length)).toBe(1);
 const chooser=page.waitForEvent('filechooser');await page.getByText('Attach',{exact:true}).click();await (await chooser).setFiles({name:'proof.txt',mimeType:'text/plain',buffer:Buffer.from('file body')});
 await page.getByLabel('Message').fill('Second with file');await page.getByText('Send',{exact:true}).click();
 await expect(page.getByLabel('Очередь сообщений')).toContainText('proof.txt');
 await page.getByLabel('Message').fill('Remove me');await page.getByText('Send',{exact:true}).click();
 await page.getByLabel('Удалить из очереди').last().click();
 await expect(page.getByLabel('Очередь сообщений')).not.toContainText('Remove me');
 await page.screenshot({path:info.outputPath('queue.png')});
 await page.evaluate(()=>window.finish());
 await expect.poll(()=>page.evaluate(()=>window.sent.length)).toBe(2);
 expect(await page.evaluate(()=>window.sent[1].attachments[0].content[0].filename)).toBe('proof.txt');
 await page.evaluate(()=>window.finish());
 await expect(page.getByLabel('Очередь сообщений')).toHaveCount(0);
});
