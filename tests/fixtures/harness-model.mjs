import {createServer} from 'node:http';

// A real OpenAI wire endpoint: scripts control model decisions, never Host results.
export async function harnessModel(){
 const requests=[],steps=[];let summaries=0;
 const server=createServer(async(req,res)=>{
  let raw='';for await(const chunk of req)raw+=chunk;
  if(req.url!=='/v1/chat/completions'){res.writeHead(404).end();return;}
  const body=JSON.parse(raw);requests.push(body);
  if(!body.stream){summaries++;res.setHeader('Content-Type','application/json');res.end(JSON.stringify({id:'summary',object:'chat.completion',created:1,model:'fixture',choices:[{index:0,message:{role:'assistant',content:`SUMMARY-${summaries}: preserve workspace decisions`},finish_reason:'stop'}],usage:{prompt_tokens:17,completion_tokens:9,total_tokens:26}}));return;}
  const step=steps.shift();
  if(!step){res.writeHead(500).end('No scripted fixture response');return;}
  res.writeHead(200,{'Content-Type':'text/event-stream','Cache-Control':'no-cache'});
  const chunk=(delta,finish=null)=>res.write(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:1,model:'fixture',choices:[{index:0,delta,finish_reason:finish}]})}\n\n`);
  chunk({role:'assistant',reasoning_content:step.reasoning??'Inspecting fixture workspace'});
  if(step.prefix)chunk({content:step.prefix});
  if(step.delay)await new Promise(resolve=>setTimeout(resolve,step.delay));
  if(res.destroyed)return;
  if(step.tool)chunk({tool_calls:[{index:0,id:step.id,type:'function',function:{name:step.tool,arguments:JSON.stringify(step.args)}}]},'tool_calls');
  else {chunk({content:step.text??'Fixture finished'});chunk({},'stop');}
  res.write(`data: ${JSON.stringify({id:'fixture',object:'chat.completion.chunk',created:1,model:'fixture',choices:[],usage:{prompt_tokens:23,completion_tokens:11,total_tokens:34}})}\n\n`);
  res.end('data: [DONE]\n\n');
 });
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 return {requests,steps,get summaries(){return summaries;},url:`http://127.0.0.1:${server.address().port}/v1`,async close(){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}};
}
