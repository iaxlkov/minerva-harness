import { ScrollArea } from '../../components/ui/scroll-area';
import type { PluginInfo } from '../../../../../packages/client/src/types.ts';
import React, { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, ArrowLeft, Boxes, Link2, PackageOpen, RotateCw, Search, ShieldCheck, Sparkles, Store, TerminalSquare } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Skeleton } from '../../components/ui/skeleton';
import { Input } from '../../components/ui/input';
import { Switch } from '../../components/ui/switch';
import { Shell } from '../../Shell';


import {models} from "../../app/models";
import {LearningPanel} from '../learning/LearningPanel';

export type PluginFilter = 'all' | 'system' | 'optional' | 'installed';
const groups: { id: Exclude<PluginFilter, 'all'>; title: string; description: string; Icon: typeof Boxes }[] = [
  { id: 'system', title: 'Системные', description: 'Ядро Minerva. Работают всегда или управляются общими настройками.', Icon: ShieldCheck },
  { id: 'optional', title: 'Дополнительные', description: 'Встроенные возможности, которые можно включить для рабочего пространства.', Icon: Sparkles },
  { id: 'installed', title: 'Установленные', description: 'Пакеты расширений, установленные отдельно.', Icon: PackageOpen },
];
const groupOf = (plugin: PluginInfo): Exclude<PluginFilter, 'all'> =>
  plugin.origin === 'builtin-optional' ? 'optional' : plugin.origin.startsWith('builtin') ? 'system' : 'installed';

// The runtime reports unmet requirements as "Enable dependencies: a, b" (crates/runtime/src/learning.rs).
const missingDependencies = (plugin: PluginInfo) => plugin.error?.match(/^Enable dependencies: (.+)$/)?.[1].split(', ') ?? null;
function status(plugin: PluginInfo): { label: string; tone: 'ok' | 'off' | 'paused' | 'error' } {
  if (missingDependencies(plugin)) return { label: 'Нужны зависимости', tone: 'paused' };
  if (plugin.canToggle && plugin.requestedEnabled && !plugin.enabled && !plugin.optionalEnabled) return { label: 'Приостановлен', tone: 'paused' };
  if (plugin.error) return { label: plugin.enabled ? 'Работает с ошибкой' : 'Ошибка запуска', tone: 'error' };
  return plugin.enabled ? { label: 'Работает', tone: 'ok' } : { label: 'Отключён', tone: 'off' };
}

export type PluginsPage = 'installed' | 'marketplace';
export function PluginsNavigation({ page, onPage }: { page: PluginsPage; onPage: (page: PluginsPage) => void }) {
  const plugins = React.useSyncExternalStore(models.plugins.subscribe, models.plugins.getSnapshot);
  const item = (id: PluginsPage, Icon: typeof Boxes, label: string, meta: React.ReactNode) => <Button variant="ghost" className={`nav-item ${page === id ? 'selected' : ''}`} aria-current={page === id ? 'page' : undefined} onClick={() => onPage(id)}>
    <Icon size={16} aria-hidden="true"/><span className="nav-item-label">{label}</span>{meta}
  </Button>;
  return <ScrollArea className="section-navigation">
    {item('installed', Boxes, 'Мои плагины', <span className="nav-item-count">{plugins.length}</span>)}
    {item('marketplace', Store, 'Маркетплейс', <span className="nav-item-badge">Скоро</span>)}
  </ScrollArea>;
}

function Marketplace() {
  return <ScrollArea asChild><section className="plugins-screen" aria-labelledby="marketplace-title">
    <div className="plugins-content">
      <div className="plugins-header"><div><h1 id="marketplace-title">Маркетплейс</h1><p>Каталог расширений для Minerva.</p></div></div>
      <div className="plugins-empty"><Store aria-hidden="true"/><strong>Маркетплейс скоро появится</strong><span>Здесь можно будет искать и устанавливать расширения. Пока пакеты устанавливаются командой <code>npm run server -- --install-plugin &lt;путь к пакету&gt;</code>.</span></div>
    </div>
  </section></ScrollArea>;
}

export function Plugins({ page = 'installed' }: { page?: PluginsPage }) {
  const [filter, setFilter] = useState<PluginFilter>('all');
  const [showShell, setShowShell] = useState(false);
  const [learningId, setLearningId] = useState<string | null>(null);
  const plugins=React.useSyncExternalStore(models.plugins.subscribe,models.plugins.getSnapshot);
  const rosterError=React.useSyncExternalStore(models.plugins.errors.subscribe,models.plugins.errors.getSnapshot);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState('');
  const refresh = useCallback(async () => {
    setError(''); setRefreshing(true);
    try { await models.plugins.refresh(); }
    catch { /* The model publishes refresh errors and clears them after recovery. */ }
    finally { setRefreshing(false); setLoaded(true); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  async function toggle(plugin: PluginInfo,enabled=plugin.optionalEnabled ? !plugin.enabled : !plugin.requestedEnabled) {
    setBusy(plugin.id); setError('');
    try { await models.plugins.toggle(plugin.id,enabled); }
    catch (cause) { setError(`Не удалось изменить состояние «${plugin.name}»: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  async function toggleOptional(enabled: boolean) {
    setBusy('plugin-manager'); setError('');
    try { await models.plugins.toggleOptional(enabled); }
    catch (cause) { setError(`Не удалось изменить состояние: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  if (learningId) {
    const plugin = plugins.find(p => p.id === learningId);
    if (!plugin?.enabled) return <section className="plugins-screen"><div className="plugins-content"><Button variant="ghost" onClick={() => setLearningId(null)}><ArrowLeft/> Назад</Button><div className="plugins-empty"><Boxes aria-hidden="true"/><strong>{plugin?.name ?? learningId}</strong><span>Включите плагин в списке, чтобы пользоваться его функциями.</span></div></div></section>;
    return <LearningPanel pluginId={learningId} onBack={() => setLearningId(null)}/>;
  }
  if (showShell) return <Shell onBack={() => setShowShell(false)}/>;
  if (page === 'marketplace') return <Marketplace/>;
  const loading = !loaded && plugins.length === 0;
  const working = plugins.filter(p => p.enabled).length, failing = plugins.filter(p => p.error && !missingDependencies(p)).length;
  async function enableDependencies(plugin: PluginInfo, ids: string[]) {
    setBusy(plugin.id); setError('');
    try { for (const id of ids) if (!plugins.find(p => p.id === id)?.requestedEnabled) await models.plugins.toggle(id, true); }
    catch (cause) { setError(`Не удалось включить зависимости «${plugin.name}»: ${String(cause)}`); }
    finally { setBusy(null); }
  }
  const needle = query.trim().toLowerCase();
  const matches = (plugin: PluginInfo) => !needle || [plugin.name, plugin.id, plugin.description].some(text => text.toLowerCase().includes(needle));
  const card = (plugin: PluginInfo) => {
    const state = status(plugin);
    const pending = busy === plugin.id;
    const on = plugin.enabled || (!plugin.optionalEnabled && plugin.requestedEnabled);
    const deps = missingDependencies(plugin);
    return <article key={plugin.id} className={`plugin-row plugin-card ${state.tone === 'error' ? 'has-error' : ''}`} aria-labelledby={`plugin-${plugin.id}`} aria-busy={pending}>
      <div className="plugin-card-head">
        <div className="plugin-icon" aria-hidden="true">{plugin.id === 'shell' ? <TerminalSquare size={16}/> : <Boxes size={16}/>}</div>
        <div className="plugin-name" id={`plugin-${plugin.id}`}>{plugin.name}{!plugin.canToggle && plugin.id !== 'plugin-manager' && <span className="plugin-badge">Обязательный</span>}</div>
      </div>
      {plugin.description && <p className="plugin-description" title={plugin.description}>{plugin.description}</p>}
      {deps ? <p className="plugin-notice"><Link2 size={14} aria-hidden="true"/><span>Работает только вместе с: {deps.map(id => plugins.find(p => p.id === id)?.name ?? id).join(', ')}.</span></p>
        : plugin.error && <p className="plugin-error" role="alert"><AlertTriangle size={14} aria-hidden="true"/><span>{plugin.error}</span></p>}
      <div className="plugin-card-foot">
        <div className={`plugin-status tone-${state.tone}`}><span className="status-dot" aria-hidden="true"/>{state.label}</div>
        <div className="plugin-actions">
          {plugin.origin === 'builtin-optional' && <Button variant="ghost" size="sm" onClick={() => setLearningId(plugin.id)}>Открыть</Button>}
          {plugin.id === 'plugin-manager' ? <label className="plugin-switch"><span>Несистемные</span><Switch aria-label="Несистемные плагины" title={plugin.optionalEnabled ? 'Приостановить несистемные плагины' : 'Включить несистемные плагины'} checked={plugin.optionalEnabled} aria-busy={pending} disabled={busy !== null} onCheckedChange={enabled => void toggleOptional(enabled)}/></label>
            : plugin.id === "shell" ? <Button variant="outline" size="sm" onClick={() => setShowShell(true)}>Настроить</Button>
            : plugin.canToggle ? <>
              {deps && <Button variant="outline" size="sm" disabled={busy !== null} onClick={() => void enableDependencies(plugin, deps)}>Включить зависимости</Button>}
              {plugin.error && !deps && !plugin.enabled && plugin.optionalEnabled && <Button variant="ghost" size="sm" disabled={busy !== null} onClick={()=>void toggle(plugin,false)}>Отключить автозапуск</Button>}
              <Switch aria-label={plugin.name} title={on ? 'Отключить' : plugin.error && plugin.optionalEnabled ? 'Повторить запуск' : 'Включить'} checked={on} aria-busy={pending} disabled={busy !== null} onCheckedChange={() => void toggle(plugin)}/>
            </> : null}
        </div>
      </div>
    </article>;
  };
  const visible = groups.filter(group => filter === 'all' || filter === group.id).map(group => ({ ...group, items: plugins.filter(p => groupOf(p) === group.id && matches(p)) }));
  const found = visible.reduce((sum, group) => sum + group.items.length, 0);
  return <ScrollArea asChild><section className="plugins-screen" aria-labelledby="plugins-title">
    <div className="plugins-content">
      <div className="plugins-header">
        <div><h1 id="plugins-title">Плагины</h1>{!loading && plugins.length > 0 && <p>{plugins.length} всего · {working} работают{failing ? <> · <span className="plugins-failing">{failing} с ошибкой</span></> : null}</p>}</div>
        <div className="plugins-toolbar">
          <label className="plugins-search"><Search size={14} aria-hidden="true"/><Input type="search" placeholder="Поиск плагинов" aria-label="Поиск плагинов" value={query} onChange={event => setQuery(event.target.value)}/></label>
          <Button variant="outline" size="icon-lg" onClick={() => void refresh()} disabled={refreshing} aria-busy={refreshing} aria-label="Обновить список" title="Обновить список"><RotateCw className={refreshing ? 'oe-animate-spin' : ''}/></Button>
        </div>
      </div>
      {!loading && plugins.length > 0 && <div className="plugins-filters" role="group" aria-label="Фильтр плагинов">
        {([['all', 'Все', plugins.length], ...groups.map(group => [group.id, group.title, plugins.filter(p => groupOf(p) === group.id).length])] as [PluginFilter, string, number][]).map(([id, label, count]) =>
          <Button key={id} variant="ghost" size="sm" aria-pressed={filter === id} onClick={() => setFilter(id)}>{label}<span className="plugins-filter-count">{count}</span></Button>)}
      </div>}
      {error && <div className="plugins-error" role="alert"><AlertTriangle size={16} aria-hidden="true"/><span>{error}</span><Button variant="ghost" size="sm" onClick={() => setError('')}>Скрыть</Button></div>}
      {!error && rosterError && <div className="plugins-error" role="alert"><AlertTriangle size={16} aria-hidden="true"/><span>Не удалось обновить список плагинов: {rosterError}</span><Button variant="ghost" size="sm" onClick={() => void refresh()}>Повторить</Button></div>}
      {loading && <div className="plugin-grid" role="status" aria-label="Загружаем плагины…">{[0, 1, 2, 3, 4, 5].map(i => <div key={i} className="plugin-card"><div className="plugin-card-head"><Skeleton className="plugin-icon"/><Skeleton className="oe-h-4 oe-w-32"/></div><Skeleton className="oe-h-3 oe-w-full"/><Skeleton className="oe-h-3 oe-w-2/3"/></div>)}</div>}
      {!loading && plugins.length === 0 && <div className="plugins-empty"><PackageOpen aria-hidden="true"/><strong>Пока нет подключённых плагинов</strong><span>Обновите список или установите расширение.</span></div>}
      {!loading && plugins.length > 0 && needle && found === 0 && <div className="plugins-empty"><Search aria-hidden="true"/><strong>Ничего не найдено</strong><span>Нет плагинов по запросу «{query.trim()}».</span><Button variant="outline" size="sm" onClick={() => setQuery('')}>Сбросить поиск</Button></div>}
      {!loading && plugins.length > 0 && visible.map(group => {
        if (!group.items.length && (needle || (filter === 'all' && group.id !== 'installed'))) return null;
        return <section key={group.id} className="plugin-group" aria-labelledby={`plugin-group-${group.id}`}>
          <header className="plugin-group-header"><group.Icon size={14} aria-hidden="true"/><h2 id={`plugin-group-${group.id}`}>{group.title}</h2><span className="plugin-group-count">{group.items.length}</span><p>{group.description}</p></header>
          {group.items.length ? <div className="plugin-grid">{group.items.map(card)}</div>
            : <div className="plugins-empty"><PackageOpen aria-hidden="true"/><strong>{group.id === 'installed' ? 'Расширения пока не установлены' : 'В этой группе нет плагинов'}</strong>{group.id === 'installed' && <span>Установите пакет командой <code>npm run server -- --install-plugin &lt;путь к пакету&gt;</code> и обновите список.</span>}</div>}
        </section>;
      })}
      <p className="plugins-note">Установленные расширения включаются отдельно для этого рабочего пространства.</p>
    </div>
  </section></ScrollArea>;
}
