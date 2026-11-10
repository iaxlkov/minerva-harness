import { ScrollArea } from '../../components/ui/scroll-area';
import type { PluginInfo } from '../../../../../packages/client/src/types.ts';
import React, { useCallback, useEffect, useState } from 'react';
import { Boxes, RotateCw } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Shell } from '../../Shell';


import {models} from "../../app/models";
import {LearningPanel} from '../learning/LearningPanel';

export function Plugins() {
  const [showShell, setShowShell] = useState(false);
  const [learningId, setLearningId] = useState<string | null>(null);
  const plugins=React.useSyncExternalStore(models.plugins.subscribe,models.plugins.getSnapshot);
  const rosterError=React.useSyncExternalStore(models.plugins.errors.subscribe,models.plugins.errors.getSnapshot);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    setError('');
    try { await models.plugins.refresh(); }
    catch { /* The model publishes refresh errors and clears them after recovery. */ }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  async function toggle(plugin: PluginInfo,enabled=plugin.optionalEnabled ? !plugin.enabled : !plugin.requestedEnabled) {
    setBusy(plugin.id); setError('');
    try { await models.plugins.toggle(plugin.id,enabled); }
    catch (cause) { setError(`Не удалось изменить состояние: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  async function toggleOptional(enabled: boolean) {
    setBusy('plugin-manager'); setError('');
    try { await models.plugins.toggleOptional(enabled); }
    catch (cause) { setError(`Не удалось изменить состояние: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  if (learningId) {
    const plugin = plugins?.find(p => p.id === learningId);
    if (!plugin?.enabled) return <section className="plugins-screen"><Button variant="outline" onClick={() => setLearningId(null)}>Назад</Button><h1>{plugin?.name ?? learningId}</h1><p>Включите плагин в списке, чтобы пользоваться его функциями.</p></section>;
    return <LearningPanel pluginId={learningId} onBack={() => setLearningId(null)}/>;
  }
  if (showShell) return <Shell onBack={() => setShowShell(false)}/>;
  return <ScrollArea asChild><section className="plugins-screen" aria-labelledby="plugins-title">
    <div className="plugins-header"><div><h1 id="plugins-title">Плагины</h1><p>Встроенные и установленные расширения рабочего пространства.</p></div><Button variant="outline" onClick={() => void refresh()} aria-label="Обновить список"><RotateCw data-icon="inline-start"/> Обновить</Button></div>
    {error && <div className="plugins-error" role="alert">{error}</div>}
    {!error && rosterError && <div className="plugins-error" role="alert">Не удалось обновить список плагинов: {rosterError}</div>}
    {plugins === null && !error && <div className="plugins-empty" role="status">Загружаем плагины…</div>}
    {plugins !== null && plugins.length === 0 && <div className="plugins-empty">Пока нет подключённых плагинов.</div>}
    {plugins && <div className="plugin-list">{plugins.map(plugin => <article key={plugin.id} className="plugin-row">
      <div className="plugin-icon" aria-hidden="true"><Boxes size={18}/></div>
      <div className="plugin-details"><div className="plugin-name">{plugin.name}<span className="plugin-badge">{plugin.origin.startsWith('builtin')?'Встроенный':'Установленный'}</span></div><p>{plugin.description}</p><div className="plugin-status"><span className={plugin.enabled ? 'status-dot' : 'status-dot unavailable'}/>{plugin.canToggle && plugin.requestedEnabled && !plugin.enabled && !plugin.optionalEnabled ? 'Приостановлен' : plugin.error ? (plugin.enabled ? 'Работает с ошибкой' : 'Ошибка запуска') : plugin.enabled ? 'Работает' : 'Отключён'}</div>{plugin.error && <p className="plugins-error" role="alert">{plugin.error}</p>}</div>
      {plugin.origin === 'builtin-optional' && <Button variant="outline" onClick={() => setLearningId(plugin.id)}>Открыть</Button>}
      {plugin.id === 'plugin-manager' ? <Button variant="outline" role="switch" aria-label="Несистемные плагины" aria-checked={plugin.optionalEnabled} aria-busy={busy === plugin.id} disabled={busy !== null} onClick={() => void toggleOptional(!plugin.optionalEnabled)}>Несистемные плагины: {busy === plugin.id ? 'Подождите…' : plugin.optionalEnabled ? 'включены' : 'приостановлены'}</Button> : plugin.id === "shell" ? <Button variant="outline" onClick={() => setShowShell(true)}>Настроить</Button> : plugin.canToggle ? <><Button variant="outline" disabled={busy !== null} onClick={() => void toggle(plugin)}>{busy === plugin.id ? 'Подождите…' : plugin.enabled || (!plugin.optionalEnabled && plugin.requestedEnabled) ? 'Отключить' : plugin.error && plugin.optionalEnabled ? 'Повторить запуск' : 'Включить'}</Button>{plugin.error && !plugin.enabled && plugin.optionalEnabled && <Button variant="outline" disabled={busy !== null} onClick={()=>void toggle(plugin,false)}>Отключить автозапуск</Button>}</> : <span className="plugin-required">Обязательный</span>}
    </article>)}</div>}
    <p className="plugins-note">Установленные расширения включаются отдельно для этого рабочего пространства.</p>
  </section></ScrollArea>;
}
