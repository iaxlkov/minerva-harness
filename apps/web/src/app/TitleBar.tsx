import React, { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Code2, Copy, Menu, MessageSquare, Minus, MoreHorizontal, PanelLeft, Square, X } from "lucide-react";
import { Button } from "../components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "../components/ui/dropdown-menu";
import "./title-bar.css";

const tauri = "__TAURI_INTERNALS__" in globalThis;
// macOS keeps native traffic lights through titleBarStyle "Overlay" (tauri.macos.conf.json).
const mac = /Mac/.test(navigator.userAgent);

function WindowControls() {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    const win = getCurrentWindow();
    const sync = () => void win.isMaximized().then(setMaximized).catch(() => {});
    sync();
    const unlisten = win.onResized(sync);
    return () => void unlisten.then((stop) => stop());
  }, []);
  const win = () => getCurrentWindow();
  return (
    <div className="window-controls">
      <button type="button" aria-label="Свернуть окно" onClick={() => void win().minimize()}>
        <Minus />
      </button>
      <button
        type="button"
        aria-label={maximized ? "Восстановить окно" : "Развернуть окно"}
        onClick={() => void win().toggleMaximize()}
      >
        {maximized ? <Copy className="window-restore" /> : <Square />}
      </button>
      <button type="button" className="window-close" aria-label="Закрыть окно" onClick={() => void win().close()}>
        <X />
      </button>
    </div>
  );
}

export function TitleBar({
  menu,
  start,
  title,
  actions,
  end,
  more,
  sidebarOpen,
  onSidebar,
  devMode,
  onDevMode,
}: {
  menu?: React.ReactNode;
  start?: React.ReactNode;
  title: React.ReactNode;
  actions?: React.ReactNode;
  end?: React.ReactNode;
  more?: React.ReactNode;
  sidebarOpen?: boolean;
  onSidebar?: () => void;
  devMode?: boolean;
  onDevMode?: (on: boolean) => void;
}) {
  return (
    <header className={`titlebar ${tauri && mac ? "titlebar-mac" : ""}`} data-tauri-drag-region>
      <div className="titlebar-lead">
      <div className="titlebar-group">
        {menu && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-lg" className="titlebar-button" aria-label="Меню приложения">
                <Menu />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">{menu}</DropdownMenuContent>
          </DropdownMenu>
        )}
        {onSidebar && (
          <Button
            variant="ghost"
            size="icon-lg"
            className="titlebar-button"
            aria-label="Боковая панель"
            title="Боковая панель"
            aria-expanded={!!sidebarOpen}
            aria-controls="section-sidebar"
            onClick={onSidebar}
          >
            <PanelLeft />
          </Button>
        )}
        {start}
      </div>
      {onDevMode && (
        <div className="titlebar-modes" role="group" aria-label="Режим">
          <Button variant="ghost" size="icon" aria-label="Чат" title="Чат" aria-pressed={!devMode} onClick={() => onDevMode(false)}>
            <MessageSquare />
          </Button>
          <Button variant="ghost" size="icon" aria-label="Режим разработки" title="Режим разработки" aria-pressed={!!devMode} onClick={() => onDevMode(true)}>
            <Code2 />
          </Button>
        </div>
      )}
      </div>
      <div className="titlebar-title breadcrumbs" data-tauri-drag-region>
        {title}
      </div>
      <div className="titlebar-group titlebar-actions shell-header-actions">
        {actions}
        {end}
        {more && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-lg" className="titlebar-button" aria-label="Ещё">
                <MoreHorizontal />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="titlebar-more-menu">{more}</DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {tauri && !mac && <WindowControls />}
    </header>
  );
}
