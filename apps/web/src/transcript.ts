import type { ThreadAssistantMessage } from '@assistant-ui/react';

import type { ContentPart } from '../../../packages/client/src/types.ts';
export type { ContentPart } from '../../../packages/client/src/types.ts';
import {jsonObject} from './json.ts';

// Both IPC results and reloaded messages use this same projection. Results are
// joined by call ID, never by array position (parallel calls finish out of order).
export function transcriptContent(parts: ContentPart[]): ThreadAssistantMessage['content'] {
  const results = new Map(parts.filter(p => p.kind === 'tool-result').map(p => {
    const data=jsonObject(p.data);if(!data||typeof data.callId!=='string')throw Error('Некорректный результат инструмента');
    return [data.callId,data] as const;
  }));
  return parts.flatMap(part => {
    const data = jsonObject(part.data);
    switch (part.kind) {
      case 'tool-result': return [];
      case 'tool-call': {
        if(!data||typeof data.callId!=='string'||typeof data.name!=='string')throw Error('Некорректный вызов инструмента');
        const result = results.get(data.callId);
        const args = data.arguments && typeof data.arguments === 'object' && !Array.isArray(data.arguments) ? data.arguments : { input: data.arguments };
        const status=result?(data.status==='cancelled'||data.status==='failed'?data.status:result.isError?'failed':'completed'):data.status;
        return [{ type: 'tool-call', toolCallId: data.callId, toolName: data.name, args, argsText: JSON.stringify(args), providerMetadata:{minerva:{...(status!==undefined?{status}:{}),...(data.meta!==undefined?{meta:data.meta}:{}),...(result?.meta!==undefined?{resultMeta:result.meta}:{})}}, ...(result ? { result: { output: result.output }, isError: result.isError } : {}) }];
      }
      case 'file': {
        if(!data||typeof data.id!=='string'||typeof data.name!=='string'||typeof data.bytes!=='number'||typeof data.mimeType!=='string')throw Error('Некорректное вложение истории');
        const {id,name,mimeType,bytes}=data;
        return [{type:'file',data:'',filename:name,mimeType,providerMetadata:{minerva:{attachment:{id,name,mimeType,bytes}}}}];
      }
      case 'image': {
        if(!data||typeof data.id!=='string'||typeof data.name!=='string'||typeof data.bytes!=='number'||typeof data.mimeType!=='string'||!['image/png','image/jpeg','image/gif','image/webp'].includes(data.mimeType))throw Error('Некорректное изображение истории');
        const {id,name,mimeType,bytes}=data;
        return [{type:'image',image:typeof data.data==='string'?`data:${mimeType};base64,${data.data}`:'',providerMetadata:{minerva:{attachment:{id,name,mimeType,bytes}}}}];
      }
      case 'text': case 'reasoning': {
        if(!data||typeof data.text!=='string')throw Error('Некорректный текст истории');
        return [{...data,type: part.kind}];
      }
      case 'extension':
        // Legacy assistant-ui parts retain their original shape. Unknown adapter
        // items stay visible as data; never interpret them as executable UI.
        return data&&typeof data.type === 'string' ? [data] : [{type:'data', name:'provider-item', data:part.data}];
      default: throw new Error(`Неизвестный тип истории: ${part.kind}`);
    }
  }) as unknown as ThreadAssistantMessage['content'];
}
