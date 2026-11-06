import { transcriptContent, type ContentPart } from './transcript.ts';
import type { ChatModelAdapter } from '@assistant-ui/react';
import type { SessionHistoryAdapter } from './sessionHistory';

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
type Reply = { parts: ContentPart[] };

// IPC belongs here; the Rust command owns execution and durable assistant writes.
export function chatAdapter(config: {
  invoke: Invoke;
  sessionId: string;
  model: string;
  options: Record<string, unknown>;
  history: SessionHistoryAdapter;
  onRunning: (value: boolean) => void;
  onSaved: () => void;
  onError: (message: string) => void;
}): ChatModelAdapter {
  return {
    async run({ messages, abortSignal, unstable_assistantMessageId, unstable_getMessage }) {
      const response = unstable_getMessage();
      const runId = unstable_assistantMessageId ?? response.id;
      let settled = false;
      let cancelling = false;
      const cancel = async () => {
        if (cancelling) return;
        cancelling = true;
        // Cancellation may reach IPC before the run registers itself. Retry only
        // cancellation, never the model request, until it is accepted or settled.
        while (!settled) {
          try { if (await config.invoke<boolean>('session_cancel', { id: config.sessionId, runId })) return; }
          catch (error) { config.onError(`Не удалось отменить запрос: ${String(error)}`); return; }
          await new Promise(resolve => setTimeout(resolve, 50));
        }
      };
      config.onRunning(true);
      try {
        const last = messages.at(-1);
        if (!last || last.role !== 'user') throw new Error('Нужно сообщение пользователя.');
        config.history.beginResponse(runId,last.id);
        abortSignal.throwIfAborted();
        await config.history.append({ parentId: messages.at(-2)?.id ?? null, message: last });
        abortSignal.throwIfAborted();
        abortSignal.addEventListener('abort', cancel, { once: true });
        const result = await config.invoke<Reply>('session_run', {
          id: config.sessionId,
          request: { model: config.model, lastMessageId: last.id, responseId: runId, createdAt: response.createdAt.toISOString(), options: config.options },
        });
        config.history.completeResponse(runId);
        config.onSaved();
        return { content: transcriptContent(result.parts) };

      } catch (error) {
        if (!abortSignal.aborted) config.onError(`Не удалось получить ответ: ${String(error)}`);
        // A failed request can already have durable tool effects. Recover that
        // snapshot instead of dropping it or manufacturing an assistant answer.
        try {
          const saved=await config.history.recoverResponse(runId);
          if(saved?.message.role==='assistant')return {content:saved.message.content,status:saved.message.status};
        } catch(recoveryError) {config.onError(`Не удалось восстановить ход: ${String(recoveryError)}`);}
        throw error;
      } finally {
        settled = true;
        abortSignal.removeEventListener('abort', cancel);
        config.onRunning(false);
      }
    },
  };
}
