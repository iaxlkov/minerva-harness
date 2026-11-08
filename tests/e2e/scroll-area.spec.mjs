import {test,expect} from '@playwright/test';
import {build} from 'esbuild';
import {readFile} from 'node:fs/promises';

test('shared scroll area supports both axes, keyboard and forwarded refs',async({page})=>{
 const bundle=await build({stdin:{resolveDir:process.cwd(),loader:'tsx',contents:`
  import React from 'react';import {createRoot} from 'react-dom/client';
  import {ScrollArea} from './apps/web/src/components/ui/scroll-area';
  createRoot(document.getElementById('root')).render(<ScrollArea asChild><pre aria-label="Output" tabIndex={0} ref={element=>{if(element)element.dataset.ref='attached';}} style={{width:200,height:100}}>{('x'.repeat(200)+'\\n').repeat(100)}</pre></ScrollArea>);
 `},bundle:true,format:'iife',write:false});
 await page.setContent('<style>'+await readFile('apps/web/src/style.css','utf8')+'</style><div id="root" class="oe-studio"></div>');
 await page.addScriptTag({content:bundle.outputFiles[0].text});
 const area=page.getByLabel('Output');
 await expect(area).toHaveAttribute('data-ref','attached');
 expect(await area.evaluate(el=>getComputedStyle(el).scrollbarWidth)).toBe('thin');
 await area.focus();await page.keyboard.press('PageDown');
 await expect.poll(()=>area.evaluate(el=>el.scrollTop)).toBeGreaterThan(0);
 await area.evaluate(el=>el.scrollLeft=100);
 expect(await area.evaluate(el=>el.scrollLeft)).toBe(100);
});
