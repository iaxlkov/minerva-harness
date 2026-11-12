import { ScrollArea } from './components/ui/scroll-area';
import type { ShellSettings as Settings, ShellResult as Result } from '../../../packages/client/src/types.ts';
import React, { useEffect, useState } from 'react';
import { invoke } from './client.ts';
import { ArrowLeft, TerminalSquare } from 'lucide-react';
import { Button } from './components/ui/button';
import { Input } from './components/ui/input';
// The plugin runs powershell.exe on Windows and sh elsewhere.
const shellName = /Win/.test(navigator.userAgent) ? 'PowerShell' : 'sh';

export function Shell({ onBack }: { onBack: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saved, setSaved] = useState<Settings | null>(null);
  const [command, setCommand] = useState('');
  const [result, setResult] = useState<Result | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { invoke('shell_settings').then(value => { setSettings(value); setSaved(value); }).catch(cause => setError(String(cause))); }, []);
  const changed = settings && saved && (settings.timeoutMs !== saved.timeoutMs || settings.maxOutputBytes !== saved.maxOutputBytes);
  async function save() {
    if (!settings) return;
    setBusy(true); setError('');
    try { const value = await invoke('save_shell_settings', { settings }); setSettings(value); setSaved(value); }
    catch (cause) { setError(String(cause)); } finally { setBusy(false); }
  }
  async function run(event: React.FormEvent) {
    event.preventDefault(); if (!command.trim()) return;
    setBusy(true); setError(''); setResult(null);
    try { setResult(await invoke('run_shell_command', { command })); }
    catch (cause) { setError(String(cause)); } finally { setBusy(false); }
  }
  return <ScrollArea asChild><section className="shell-screen" aria-labelledby="shell-title">
    <div className="shell-content"><Button variant="ghost" type="button" className="shell-back" onClick={onBack}><ArrowLeft size={15}/> Плагины</Button>
      <div className="shell-heading"><div className="shell-icon"><TerminalSquare size={21}/></div><div><h1 id="shell-title">Командная оболочка</h1><p>Настройки встроенного плагина Cordis.</p></div></div>
      {error && <div className="shell-error" role="alert">{error}</div>}
      {!settings && !error && <div role="status">Загрузка настроек…</div>}
      {settings && <div className="shell-form provider-form">
        <div className="ds-field-row"><label>Тайм-аут команды (мс)<Input type="number" min="100" max="3600000" value={settings.timeoutMs} onChange={e => setSettings({ ...settings, timeoutMs: Number(e.target.value) })} disabled={busy}/><span className="ds-field-hint">От 100 до 3 600 000 мс.</span></label>
        <label>Лимит вывода на поток (байт)<Input type="number" min="256" max="1000000" value={settings.maxOutputBytes} onChange={e => setSettings({ ...settings, maxOutputBytes: Number(e.target.value) })} disabled={busy}/><span className="ds-field-hint">Вывод сверх лимита сохраняется во временный файл.</span></label></div>
        <div className="ds-form-actions"><Button onClick={() => void save()} disabled={!changed || busy}>Сохранить</Button></div>
      </div>}
      <div className="shell-console"><div className="shell-console-title"><TerminalSquare size={16}/> Проверка команды <span>{shellName} · один запуск</span></div>
        <form onSubmit={run} className="shell-command"><span aria-hidden="true">›</span><Input aria-label={`Команда ${shellName}`} placeholder={`Введите команду ${shellName}…`} value={command} onChange={e => setCommand(e.target.value)} disabled={busy}/><Button type="submit" disabled={busy || !command.trim()}>{busy ? 'Выполняется…' : 'Выполнить'}</Button></form>
        {result && <div className="shell-result" role="status"><div className="shell-result-meta">{result.timedOut ? 'Превышен тайм-аут оболочки' : `Код завершения: ${result.exitCode ?? 'неизвестен'}`}</div>{result.stdout && <ScrollArea asChild><pre>{result.stdout}</pre></ScrollArea>}{result.stderr && <ScrollArea asChild><pre className="shell-stderr">{result.stderr}</pre></ScrollArea>}{result.stdoutFile && <p>Полный stdout: {result.stdoutFile}</p>}{result.stderrFile && <p>Полный stderr: {result.stderrFile}</p>}</div>}
      </div><p className="shell-note">Команды запускаются вручную с правами текущего пользователя. Тайм-аут завершает процесс оболочки; дочерние процессы могут продолжить работу. Интерактивный терминал и вызов агентом пока не подключены.</p>
    </div>
  </section></ScrollArea>;
}
