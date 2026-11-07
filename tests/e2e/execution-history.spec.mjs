import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import {readFile} from 'node:fs/promises';

const image='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=';
const call=(id,name,args,status='running')=>({kind:'tool-call',data:{callId:id,name,arguments:args,status,meta:{native:{signature:'provider-signature'}}}});
const result=(id,output,isError=false)=>({kind:'tool-result',data:{callId:id,output,isError,meta:{recorded:true}}});
const reasoning={kind:'reasoning',data:{text:'Inspect the workspace before changing it.'}};
const saved=(parts,status='running')=>({id:'assistant',parentId:null,role:'assistant',createdAt:'2026-11-07T00:00:00Z',parts,extensions:{execution:{status}}});

test('actual message parts retain live tool states, reasoning, cards and images through reload',async({page})=>{
  const bundle=await build({stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
    import React from 'react';
    import {createRoot} from 'react-dom/client';
    import {AssistantRuntimeProvider,useLocalRuntime,ThreadPrimitive,MessagePrimitive,AuiIf} from '@assistant-ui/react';
    import {ToolHistory} from './apps/web/src/ToolHistory';
    import {ExecutionImage} from './apps/web/src/execution/ExecutionImage';
    import {decodeSessionMessage} from './apps/web/src/sessionHistory';
    const initial=${JSON.stringify(saved([reasoning,call('shell','shell',{command:'echo hello'},'pending')]))};
    function Fixture(){
      const runtime=useLocalRuntime({async run(){throw Error('Fixture must never execute tools or models');}});
      React.useEffect(()=>{
        window.showSnapshot=snapshot=>{localStorage.setItem('snapshot',JSON.stringify(snapshot));runtime.thread.import({messages:[decodeSessionMessage(snapshot)]});};
        window.showSnapshot(JSON.parse(localStorage.getItem('snapshot')||'null')||initial);
      },[runtime]);
      return <AssistantRuntimeProvider runtime={runtime}><ThreadPrimitive.Root><ThreadPrimitive.Messages>{()=>
        <MessagePrimitive.Root><MessagePrimitive.Parts components={{tools:{Fallback:ToolHistory},Image:ExecutionImage,Reasoning:({text})=><details className="message-reasoning"><summary>Reasoning</summary>{text}</details>}}/>
        <AuiIf condition={s=>s.message.status?.type==='incomplete'}><p className="message-interrupted">Interrupted execution</p></AuiIf>
        </MessagePrimitive.Root>
      }</ThreadPrimitive.Messages></ThreadPrimitive.Root></AssistantRuntimeProvider>;
    }
    createRoot(document.getElementById('root')).render(<Fixture/>);
  `},bundle:true,format:'esm',write:false,loader:{'.css':'empty'}});
  const css=await readFile('apps/web/src/execution/execution.css','utf8');
  await page.route('http://execution.test/**',route=>route.fulfill({contentType:route.request().url().endsWith('app.js')?'text/javascript':'text/html',body:route.request().url().endsWith('app.js')?bundle.outputFiles[0].text:`<style>${css}</style><div id="root"></div><script type="module" src="/app.js"></script>`}));
  await page.goto('http://execution.test/');
  const card=id=>page.locator(`[data-call-id="${id}"]`);
  await expect(card('shell')).toHaveAttribute('data-tool-state','pending');
  await card('shell').locator(':scope > summary').click();
  await expect(card('shell')).toContainText('Ожидание результата инструмента.');
  await expect(card('shell')).not.toContainText('прервано');
  await page.locator('.message-reasoning summary').click();
  await expect(page.locator('.message-reasoning')).toContainText(reasoning.data.text);
  await page.reload();
  await expect(card('shell')).toHaveAttribute('data-tool-state','pending');
  await expect(page.locator('.message-interrupted')).toHaveCount(0);
  await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved([reasoning,call('shell','shell',{command:'echo hello'},'requires-approval')]));
  await expect(card('shell')).toHaveAttribute('data-tool-state','requires-approval');
  await expect(card('shell').getByRole('status')).toHaveText('Требуется разрешение');
  await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved([reasoning,call('shell','shell',{command:'echo hello'})]));
  await expect(card('shell')).toHaveAttribute('data-tool-state','running');
  await page.reload();
  await expect(card('shell')).toHaveAttribute('data-tool-state','running');
  await expect(page.locator('.message-interrupted')).toHaveCount(0);
  const progress=call('shell','shell',{command:'echo hello'});
  progress.data.meta={stdout:'live shell prefix',stderr:''};
  await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved([reasoning,progress]));
  await card('shell').locator(':scope > summary').click();
  await expect(card('shell').getByLabel('Стандартный вывод')).toHaveText('live shell prefix');
  await expect(card('shell')).toHaveAttribute('data-tool-state','running');

  const parts=[reasoning,call('shell','shell',{command:'echo hello'}),call('read','read_file',{path:'a.txt'}),call('search','search_files',{pattern:'hello'}),call('diff','patch_file',{path:'a.txt'}),call('unknown','plugin.unknown',{flag:true}),
    result('read',{path:'a.txt',content:'hello file',truncated:true}),result('unknown',null),result('shell',{stdout:'hello shell',stderr:'warning',exitCode:0}),result('search',{matches:[{path:'a.txt',line:2,text:'hello match'}]}),result('diff',{diff:'-before\n+after'}),
    {kind:'image',data:{id:'image-1',name:'pixel.png',mimeType:'image/png',bytes:68,data:image}}, {kind:'text',data:{text:'Finished after the tools.'}}];
  await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved(parts,'completed'));
  await expect(page.locator('.execution-card')).toHaveCount(5);
  for(const id of ['shell','read','search','diff','unknown']){
    await expect(card(id)).toHaveAttribute('data-tool-state','completed');
    if(await card(id).getAttribute('open')===null)await card(id).locator(':scope > summary').click();
  }
  await expect(card('shell').getByLabel('Стандартный вывод')).toHaveText('hello shell');
  await expect(card('shell').getByLabel('Вывод ошибок')).toHaveText('warning');
  await expect(card('shell')).toContainText('Код выхода: 0');
  await expect(card('read').getByLabel('Содержимое файла')).toHaveText('hello file');
  await expect(card('read')).toContainText('Вывод сокращён');
  await expect(card('search')).toContainText('a.txt:2');
  await expect(card('diff').locator('[data-change="added"]')).toContainText('+after');
  await expect(card('unknown')).toContainText('null');
  await card('shell').getByText('Метаданные',{exact:true}).click();
  await expect(card('shell')).toContainText('provider-signature');
  await expect(page.getByRole('img',{name:'pixel.png'})).toBeVisible();
  await expect(page.getByText('Finished after the tools.')).toBeVisible();
  await page.reload();
  await expect(card('shell')).toHaveAttribute('data-tool-state','completed');
  await expect(page.getByRole('img',{name:'pixel.png'})).toBeVisible();

  await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved([reasoning,call('error','shell',{command:'fail'}),result('error','offline',true)],'failed'));
  await expect(card('error')).toHaveAttribute('data-tool-state','failed');
  await expect(page.locator('.message-interrupted')).toBeVisible();
  await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved([reasoning,call('cancel','shell',{command:'wait'},'cancelled')],'cancelled'));
  await expect(card('cancel')).toHaveAttribute('data-tool-state','cancelled');
  await page.reload();
  await expect(card('cancel')).toHaveAttribute('data-tool-state','cancelled');
  await expect(page.locator('.message-interrupted')).toBeVisible();
  for(const status of ['running','requires-approval']){
    await page.evaluate(snapshot=>window.showSnapshot(snapshot),saved([call('crashed','shell',{command:'wait'},status)],'interrupted'));
    await expect(card('crashed')).toHaveAttribute('data-tool-state','interrupted');
    await page.reload();
    await expect(card('crashed')).toHaveAttribute('data-tool-state','interrupted');
  }
});
