import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Boxes, RotateCw } from 'lucide-react';
import { Button } from './components/ui/button';
import { Shell } from './Shell';

type PluginInfo = { id: string; name: string; description: string; enabled: boolean; canToggle: boolean; origin: string };

export function Plugins() {
  const [showShell, setShowShell] = useState(false);
  const [plugins, setPlugins] = useState<PluginInfo[] | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    setError('');
    try { setPlugins(await invoke<PluginInfo[]>('list_plugins')); }
    catch (cause) { setError(`Не удалось получить список плагинов: ${String(cause)}`); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  async function toggle(plugin: PluginInfo) {
    setBusy(plugin.id); setError('');
    try { setPlugins(await invoke<PluginInfo[]>('set_plugin_enabled', { id: plugin.id, enabled: !plugin.enabled })); }
    catch (cause) { setError(`Не удалось изменить состояние: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  if (showShell) return <Shell onBack={() => setShowShell(false)}/>;
  return <section className="plugins-screen" aria-labelledby="plugins-title">
    <div className="plugins-header"><div><h1 id="plugins-title">Плагины</h1><p>Встроенные возможности этого прототипа.</p></div><Button variant="outline" onClick={() => void refresh()} aria-label="Обновить список"><RotateCw data-icon="inline-start"/> Обновить</Button></div>
    {error && <div className="plugins-error" role="alert">{error}</div>}
    {plugins === null && !error && <div className="plugins-empty" role="status">Загружаем плагины…</div>}
    {plugins !== null && plugins.length === 0 && <div className="plugins-empty">Пока нет подключённых плагинов.</div>}
    {plugins && <div className="plugin-list">{plugins.map(plugin => <article key={plugin.id} className="plugin-row">
      <div className="plugin-icon" aria-hidden="true"><Boxes size={18}/></div>
      <div className="plugin-details"><div className="plugin-name">{plugin.name}<span className="plugin-badge">Встроенный</span></div><p>{plugin.description}</p><div className="plugin-status"><span className={plugin.enabled ? 'status-dot' : 'status-dot unavailable'}/>{plugin.enabled ? 'Работает' : 'Отключён'}</div></div>
      {plugin.id === "shell" ? <Button variant="outline" onClick={() => setShowShell(true)}>Настроить</Button> : plugin.canToggle ? <Button variant="outline" disabled={busy !== null} onClick={() => void toggle(plugin)}>{busy === plugin.id ? 'Подождите…' : plugin.enabled ? 'Отключить' : 'Включить'}</Button> : <span className="plugin-required">Обязательный</span>}
    </article>)}</div>}
    <p className="plugins-note">Состояние тестового плагина действует до закрытия приложения. Подключение внешних плагинов ещё не реализовано.</p>
  </section>;
}
