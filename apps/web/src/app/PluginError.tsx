import { CircleAlert, Power, Settings } from 'lucide-react';
import type { PluginErrorProps } from '../../../../packages/plugin-sdk/src/renderer';
import { Button } from '../components/ui/button';
import './plugin-error.css';

export function PluginError({pluginId,error,actionError,onOpenSettings,onDisable,fullPage=false}:PluginErrorProps & {fullPage?:boolean}) {
  return <section className={`oe-studio plugin-error${fullPage?' plugin-error-page':''}`} role="alert">
    <div className="plugin-error-content">
      <div className="plugin-error-icon"><CircleAlert size={24} aria-hidden="true"/></div>
      <p className="plugin-error-label">Ошибка плагина</p>
      <h1>Не удалось отобразить интерфейс</h1>
      <p className="plugin-error-description">Плагин <strong>{pluginId}</strong> завершился с ошибкой. Откройте настройки или отключите его, чтобы продолжить работу.</p>
      <pre className="plugin-error-details">{error}</pre>
      {actionError&&<p className="plugin-error-action-error">Не удалось отключить плагин: {actionError}</p>}
      <div className="plugin-error-actions">
        <Button size="lg" onClick={onOpenSettings}><Settings aria-hidden="true"/>Открыть настройки</Button>
        <Button size="lg" variant="outline" onClick={onDisable}><Power aria-hidden="true"/>Отключить плагин</Button>
      </div>
    </div>
  </section>;
}
