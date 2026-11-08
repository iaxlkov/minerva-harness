import { ScrollArea } from './components/ui/scroll-area';
﻿import type { ToolCallMessagePartProps } from '@assistant-ui/react';
import { Terminal, Check, CircleAlert, Clock, FileText, Search, FileDiff } from 'lucide-react';
import { display, object, toolCard, toolState } from './execution/toolModel';
import './execution/execution.css';

export function ToolHistory(props:ToolCallMessagePartProps) {
  const {toolName,args,argsText,result,toolCallId}=props;
  const progress=object(object(props.providerMetadata).minerva).meta;
  const output=result && typeof result==='object' && 'output' in result?result.output:result===undefined?progress:result;
  const card=toolCard(toolName,args,output), state=toolState(props);
  const hasOutput=result!==undefined||card.kind==='shell'&&(typeof card.result.stdout==='string'||typeof card.result.stderr==='string');
  const Icon=card.kind==='read'?FileText:card.kind==='search'?Search:card.kind==='diff'?FileDiff:Terminal;
  const StatusIcon=state.status==='completed'?Check:['failed','cancelled','interrupted'].includes(state.status)?CircleAlert:Clock;
  const path=card.result.path??card.input.path;
  return <details className="tool-history execution-card" data-call-id={toolCallId} data-tool-state={state.status} data-tool-kind={card.kind}>
    <summary><Icon size={15} aria-hidden="true"/><span className="tool-history-name">{toolName}</span>{typeof path==='string'&&<span className="execution-path" title={path}>{path}</span>}<span className="tool-history-status" role="status"><StatusIcon size={13} aria-hidden="true"/>{state.label}</span></summary>
    <div className="tool-history-body">
      <div className="tool-history-label">Аргументы</div><ScrollArea asChild><pre>{argsText}</pre></ScrollArea>
      {hasOutput?<ToolOutput card={card}/>:<p>{state.status==='interrupted'?'Выполнение прервано до получения результата.':state.status==='cancelled'?'Выполнение отменено.':state.status==='failed'?'Выполнение завершилось с ошибкой до получения результата.':state.status==='requires-approval'?'Ожидание разрешения на выполнение.':'Ожидание результата инструмента.'}</p>}
      {object(props.providerMetadata).minerva!==undefined&&<details className="execution-metadata"><summary>Метаданные</summary><ScrollArea asChild><pre>{display(object(props.providerMetadata).minerva)}</pre></ScrollArea></details>}
    </div>
  </details>;
}
function ToolOutput({card}:{card:ReturnType<typeof toolCard>}) {
  const {kind,input,result,output}=card;
  return <section aria-label="Результат инструмента">
    {kind==='shell'?<>
      {typeof input.command==='string'&&<ScrollArea asChild><pre className="execution-command">$ {input.command}</pre></ScrollArea>}
      {result.exitCode!==undefined&&<p>Код выхода: {display(result.exitCode)}</p>}
      {result.stdout!==undefined&&<ScrollArea asChild><pre aria-label="Стандартный вывод">{display(result.stdout)}</pre></ScrollArea>}
      {result.stderr!==undefined&&<ScrollArea asChild><pre aria-label="Вывод ошибок">{display(result.stderr)}</pre></ScrollArea>}
      {result.stdout===undefined&&result.stderr===undefined&&<ScrollArea asChild><pre>{display(output)}</pre></ScrollArea>}
    </>:kind==='read'&&typeof result.content==='string'?<ScrollArea asChild><pre className="execution-file" aria-label="Содержимое файла">{result.content}</pre></ScrollArea>
    :kind==='search'&&Array.isArray(result.matches)?result.matches.length===0?<p>Совпадений нет.</p>:<ol className="execution-matches">{result.matches.map((match,index)=>{const row=object(match);return <li key={index}><code>{display(row.path)}{row.line!==undefined?`:${display(row.line)}`:''}</code><ScrollArea asChild><pre>{display(row.text??match)}</pre></ScrollArea></li>;})}</ol>
    :kind==='diff'&&typeof result.diff==='string'?<ScrollArea asChild><pre className="execution-diff" aria-label="Изменения файла">{result.diff.split('\n').map((line,index)=><span key={index} data-change={line.startsWith('+')?'added':line.startsWith('-')?'removed':'context'}>{line}{'\n'}</span>)}</pre></ScrollArea>
    :<><div className="tool-history-label">Результат</div><ScrollArea asChild><pre>{display(output)}</pre></ScrollArea></>}
    {result.truncated===true&&<p role="note">Вывод сокращён инструментом.</p>}
  </section>;
}
