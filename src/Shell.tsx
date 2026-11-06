import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, TerminalSquare } from 'lucide-react';
import { Button } from './components/ui/button';

type Settings = { timeoutMs: number; maxOutputBytes: number };
type Result = { exitCode: number | null; timedOut: boolean; stdout: string; stderr: string; stdoutFile: string | null; stderrFile: string | null };
export function Shell({ onBack }: { onBack: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saved, setSaved] = useState<Settings | null>(null);
  const [command, setCommand] = useState('');
  const [result, setResult] = useState<Result | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { invoke<Settings>('shell_settings').then(value => { setSettings(value); setSaved(value); }).catch(cause => setError(String(cause))); }, []);
  const changed = settings && saved && (settings.timeoutMs !== saved.timeoutMs || settings.maxOutputBytes !== saved.maxOutputBytes);
  async function save() {
    if (!settings) return;
    setBusy(true); setError('');
    try { const value = await invoke<Settings>('save_shell_settings', { settings }); setSettings(value); setSaved(value); }
    catch (cause) { setError(String(cause)); } finally { setBusy(false); }
  }
  async function run(event: React.FormEvent) {
    event.preventDefault(); if (!command.trim()) return;
    setBusy(true); setError(''); setResult(null);
    try { setResult(await invoke<Result>('run_shell_command', { command })); }
    catch (cause) { setError(String(cause)); } finally { setBusy(false); }
  }
  return <section className="shell-screen" aria-labelledby="shell-title">
    <div className="shell-content"><button type="button" className="shell-back" onClick={onBack}><ArrowLeft size={15}/> Плагины</button>
      <div className="shell-heading"><div className="shell-icon"><TerminalSquare size={21}/></div><div><h1 id="shell-title">Командная оболочка</h1><p>Настройки встроенного плагина Cordis.</p></div></div>
      {error && <div className="shell-error" role="alert">{error}</div>}
      {!settings && !error && <div role="status">Загрузка настроек…</div>}
      {settings && <div className="shell-form">
        <label>Тайм-аут команды (мс)<input type="number" min="100" max="3600000" value={settings.timeoutMs} onChange={e => setSettings({ ...settings, timeoutMs: Number(e.target.value) })} disabled={busy}/><span>От 100 до 3 600 000 мс.</span></label>
        <label>Лимит вывода на поток (байт)<input type="number" min="256" max="1000000" value={settings.maxOutputBytes} onChange={e => setSettings({ ...settings, maxOutputBytes: Number(e.target.value) })} disabled={busy}/><span>Полный вывод сверх лимита сохраняется во временный файл.</span></label>
        <Button onClick={() => void save()} disabled={!changed || busy}>Сохранить</Button>
      </div>}
      <div className="shell-console"><div className="shell-console-title"><TerminalSquare size={16}/> Проверка команды <span>PowerShell · один запуск</span></div>
        <form onSubmit={run} className="shell-command"><span aria-hidden="true">›</span><input aria-label="Команда PowerShell" placeholder="Введите команду PowerShell…" value={command} onChange={e => setCommand(e.target.value)} disabled={busy}/><Button type="submit" disabled={busy || !command.trim()}>{busy ? 'Выполняется…' : 'Выполнить'}</Button></form>
        {result && <div className="shell-result" role="status"><div className="shell-result-meta">{result.timedOut ? 'Превышен тайм-аут оболочки' : `Код завершения: ${result.exitCode ?? 'неизвестен'}`}</div>{result.stdout && <pre>{result.stdout}</pre>}{result.stderr && <pre className="shell-stderr">{result.stderr}</pre>}{result.stdoutFile && <p>Полный stdout: {result.stdoutFile}</p>}{result.stderrFile && <p>Полный stderr: {result.stderrFile}</p>}</div>}
      </div><p className="shell-note">Команды запускаются вручную с правами текущего пользователя. Тайм-аут завершает процесс оболочки; дочерние процессы могут продолжить работу. Интерактивный терминал и вызов агентом пока не подключены.</p>
    </div>
  </section>;
}
