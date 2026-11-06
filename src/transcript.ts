import type { ThreadAssistantMessage } from '@assistant-ui/react';

export type ContentPart = { kind: string; data: Record<string, unknown> };

// Both IPC results and reloaded messages use this same projection. Results are
// joined by call ID, never by array position (parallel calls finish out of order).
export function transcriptContent(parts: ContentPart[]): ThreadAssistantMessage['content'] {
  const results = new Map(parts.filter(p => p.kind === 'tool-result').map(p => [p.data.callId, p.data]));
  return parts.flatMap(part => {
    const data = part.data;
    switch (part.kind) {
      case 'tool-result': return [];
      case 'tool-call': {
        const result = results.get(data.callId);
        const args = data.arguments && typeof data.arguments === 'object' && !Array.isArray(data.arguments) ? data.arguments : { input: data.arguments };
        return [{ type: 'tool-call', toolCallId: data.callId, toolName: data.name, args, argsText: JSON.stringify(args), ...(result ? { result: { output: result.output }, isError: result.isError } : {}) }];
      }
      case 'text': case 'reasoning': return [{...data,type: part.kind}];
      case 'extension':
        // Legacy assistant-ui parts retain their original shape. Unknown adapter
        // items stay visible as data; never interpret them as executable UI.
        return typeof data.type === 'string' ? [data] : [{type:'data', name:'provider-item', data}];
      default: throw new Error(`Неизвестный тип истории: ${part.kind}`);
    }
  }) as unknown as ThreadAssistantMessage['content'];
}
