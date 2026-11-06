import type { ToolCallMessagePartProps } from '@assistant-ui/react';
import { Terminal, Check, CircleAlert } from 'lucide-react';

export function ToolHistory({ toolName, argsText, result, isError }: ToolCallMessagePartProps) {
  const hasResult = result !== undefined;
  const output = result && typeof result === 'object' && 'output' in result ? result.output : result;
  const label = !hasResult ? 'Результат не получен' : isError ? 'Ошибка' : 'Завершено';
  const Icon = isError || !hasResult ? CircleAlert : Check;
  return <details className="tool-history">
    <summary><Terminal size={15}/><span className="tool-history-name">{toolName}</span><span className="tool-history-status"><Icon size={13}/>{label}</span></summary>
    <div className="tool-history-body"><div className="tool-history-label">Аргументы</div><pre>{argsText}</pre>
      <div className="tool-history-label">Результат</div>
      {hasResult ? <pre>{JSON.stringify(output, null, 2)}</pre> : <p>Выполнение было прервано до получения результата. Инструмент не запускается повторно при открытии истории.</p>}
    </div>
  </details>;
}
