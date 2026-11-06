import type { PluginInfo } from '../../../../../packages/client/src/types.ts';
import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../client.ts';
import { Boxes, RotateCw } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Shell } from '../../Shell';


import {models} from "../../app/models";

export function Plugins() {
  const [showShell, setShowShell] = useState(false);
  const plugins=React.useSyncExternalStore(models.plugins.subscribe,models.plugins.getSnapshot);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    setError('');
    try { await models.plugins.refresh(); }
    catch (cause) { setError(`Не удалось получить список плагинов: ${String(cause)}`); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  async function toggle(plugin: PluginInfo) {
    setBusy(plugin.id); setError('');
    try { await models.plugins.toggle(plugin.id,!plugin.enabled); }
    catch (cause) { setError(`Не удалось изменить состояние: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  if (showShell) return <Shell onBack={() => setShowShell(false)}/>;
  return <section className="plugins-screen" aria-labelledby="plugins-title">
    <div className="plugins-header"><div><h1 id="plugins-title">Плагины</h1><p>Встроенные и установленные расширения рабочего пространства.</p></div><Button variant="outline" onClick={() => void refresh()} aria-label="Обновить список"><RotateCw data-icon="inline-start"/> Обновить</Button></div>
    {error && <div className="plugins-error" role="alert">{error}</div>}
    {plugins === null && !error && <div className="plugins-empty" role="status">Загружаем плагины…</div>}
    {plugins !== null && plugins.length === 0 && <div className="plugins-empty">Пока нет подключённых плагинов.</div>}
    {plugins && <div className="plugin-list">{plugins.map(plugin => <article key={plugin.id} className="plugin-row">
      <div className="plugin-icon" aria-hidden="true"><Boxes size={18}/></div>
      <div className="plugin-details"><div className="plugin-name">{plugin.name}<span className="plugin-badge">{plugin.origin==='builtin'?'Встроенный':'Установленный'}</span></div><p>{plugin.description}</p><div className="plugin-status"><span className={plugin.enabled ? 'status-dot' : 'status-dot unavailable'}/>{plugin.enabled ? 'Работает' : 'Отключён'}</div></div>
      {plugin.id === "shell" ? <Button variant="outline" onClick={() => setShowShell(true)}>Настроить</Button> : plugin.canToggle ? <Button variant="outline" disabled={busy !== null} onClick={() => void toggle(plugin)}>{busy === plugin.id ? 'Подождите…' : plugin.enabled ? 'Отключить' : 'Включить'}</Button> : <span className="plugin-required">Обязательный</span>}
    </article>)}</div>}
    <p className="plugins-note">Установленные расширения включаются отдельно для этого рабочего пространства.</p>
  </section>;
}
