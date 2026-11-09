import React, {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  Home,
  Puzzle,
  Settings as SettingsIcon,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
} from "lucide-react";
import { Button } from "../components/ui/button";
import { ScrollArea } from "../components/ui/scroll-area";
import { Settings } from "../pages/settings/Settings";
import { Plugins } from "../pages/plugins/Plugins";
import { UiSlot, useUiPlugins, workspaceScope } from "./plugins";
import type {
  Provider,
  SessionInfo,
} from "../../../../packages/client/src/types";
import {
  SidebarProvider,
  Sidebar,
  SidebarHeader,
  SidebarFooter,
  SidebarContent,
} from "../components/ui/sidebar";
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "../components/ui/resizable";
import type { PanelImperativeHandle } from "react-resizable-panels";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "../components/ui/sheet";
import {
  TooltipProvider,
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from "../components/ui/tooltip";
import { useIsMobile } from "../hooks/use-mobile";
import "./navigation-shell.css";

function readPreference(key: string, fallback: number) {
  try {
    const value = localStorage.getItem(key);
    return value !== null && Number.isFinite(Number(value))
      ? Number(value)
      : fallback;
  } catch {
    return fallback;
  }
}
const clamp = (value: number) =>
  Math.round(Math.max(192, Math.min(480, value)));
export function NavigationShell({
  children,
  session,
  sessions,
  onSession,
  onNew,
  providers,
  onProviders,
  activePlugins,
}: {
  children: React.ReactNode;
  session: SessionInfo;
  sessions: SessionInfo[];
  onSession: (session: SessionInfo) => void;
  onNew: (workspaceId?: string) => void;
  providers: Provider[];
  onProviders: (providers: Provider[]) => void;
  activePlugins: string[];
}) {
  const host = useUiPlugins();
  useSyncExternalStore((callback) => {
    const sub = host.slots.subscribe(callback);
    return () => sub.dispose();
  }, host.slots.getSnapshot);
  const [section, setSection] = useState("chat");
  const [tab, setTab] = useState("models");
  const [width, setWidth] = useState(() =>
    clamp(readPreference("minerva.sidebar.width", 272)),
  );
  const [collapsed, setCollapsed] = useState(
    () => !!readPreference("minerva.sidebar.collapsed", 0),
  );
  const [mobileOpen, setMobileOpen] = useState(false);
  // Changing defaultSize re-registers the panel and interrupts an active drag.
  const initialPanelSize = useRef(collapsed ? 0 : width).current;
  const panel = useRef<PanelImperativeHandle | null>(null);
  const isMobile = useIsMobile();
  useEffect(() => {
    panel.current?.resize(isMobile || collapsed ? 0 : width);
  }, [collapsed, isMobile]);
  const sections = [
    ...host.slots.entries("shell.sections", workspaceScope),
  ].sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
  useEffect(() => {
    if (
      !["chat", "plugins", "settings"].includes(section) &&
      !sections.some((entry) => entry.entryId === section)
    )
      setSection("chat");
  }, [section, sections]);
  useEffect(() => {
    try {
      localStorage.setItem("minerva.sidebar.width", String(width));
      localStorage.setItem(
        "minerva.sidebar.collapsed",
        String(Number(collapsed)),
      );
    } catch {}
  }, [width, collapsed]);
  function navigate(next: string) {
    setSection(next);
    setMobileOpen(false);
  }
  const slotProps = { section, onNavigate: navigate };
  const sectionSlot = (
    placement: "rail" | "sidebar" | "body" | "title",
    entryId?: string,
  ) => (
    <UiSlot
      slot="shell.sections"
      scope={workspaceScope}
      entryId={entryId}
      componentProps={{
        ...slotProps,
        placement,
        selected: entryId === section,
      }}
    />
  );
  const settings = (placement: "navigation" | "body") => (
    <Settings
      placement={placement}
      tab={tab}
      onSelectPage={(id)=>{setTab(id);setMobileOpen(false);}}
      sessionId={session.id}
      providers={providers}
      onChange={onProviders}
      activePlugins={activePlugins}
    />
  );
  function railButton(id: string, label: string, Icon: typeof Home) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-lg"
            className={`rail-button ${section === id ? "selected" : ""} ${id === "settings" ? "settings-open" : ""}`}
            aria-label={label}
            aria-current={section === id ? "page" : undefined}
            onClick={() => navigate(id)}
          >
            <Icon />
          </Button>
        </TooltipTrigger>
        <TooltipContent className="oe-studio" side="right">
          {label}
        </TooltipContent>
      </Tooltip>
    );
  }
  const sidebarOpen = isMobile ? mobileOpen : !collapsed;
  const toggle = () => {
    if (isMobile) setMobileOpen((value) => !value);
    else setCollapsed((value) => !value);
  };
  const sidebarContent = (
    <>
      {" "}
      <SidebarHeader className="sidebar-heading">
        <div className="brand">
          <strong className="brand-name">
            {section === "chat"
              ? "Minerva"
              : section === "plugins"
                ? "Плагины"
                : section === "settings"
                  ? "Настройки"
                  : sectionSlot("title", section)}
          </strong>
        </div>
        <UiSlot
          slot="shell.sidebar.header"
          scope={workspaceScope}
          componentProps={slotProps}
        />
      </SidebarHeader>
      <SidebarContent>
        {section === "chat" ? (
          <>
            <Button
              variant="outline"
              className="new-chat"
              onClick={() => {
                navigate("chat");
                onNew();
              }}
            >
              <Plus /> Новая сессия
            </Button>
            <UiSlot
              slot="navigation.items"
              scope={workspaceScope}
              componentProps={{
                session,
                sessions,
                disabled: false,
                onNew: (id) => {
                  navigate("chat");
                  onNew(id);
                },
                onSession: (item) => {
                  navigate("chat");
                  onSession(item);
                },
              }}
            />
          </>
        ) : section === "settings" ? (
          settings("navigation")
        ) : section === "plugins" ? (
          <ScrollArea className="section-navigation">
            <p className="section-description">
              Управляйте возможностями Minerva и подключайте расширения.
            </p>
            <Button
              variant="ghost"
              className="nav-item selected"
              aria-current="page"
            >
              Все плагины
            </Button>
            <p className="section-description">
              Настройки подключённых плагинов доступны на странице настроек.
            </p>
          </ScrollArea>
        ) : (
          sectionSlot("sidebar", section)
        )}
        <UiSlot
          slot="shell.sidebar.content"
          scope={workspaceScope}
          componentProps={slotProps}
        />
      </SidebarContent>
      <SidebarFooter className="sidebar-bottom">
        <UiSlot
          slot="shell.sidebar.footer"
          scope={workspaceScope}
          componentProps={slotProps}
        />
      </SidebarFooter>
    </>
  );
  return (
    <TooltipProvider delayDuration={350}>
      <SidebarProvider
        open={!collapsed}
        onOpenChange={(open) => setCollapsed(!open)}
        className="shell-provider"
      >
        <div
          className={`oe-studio app-shell navigation-shell ${collapsed ? "sidebar-collapsed" : ""} ${mobileOpen ? "sidebar-mobile-open" : ""}`}
          style={{ "--sidebar-width": `${width}px` } as React.CSSProperties}
        >
          <nav className="activity-rail" aria-label="Разделы приложения">
            <img
              className="rail-logo"
              src="/minerva-64.png"
              alt="Minerva"
              width={30}
              height={30}
            />
            <UiSlot
              slot="shell.rail.start"
              scope={workspaceScope}
              componentProps={slotProps}
            />
            {railButton("chat", "Чаты", Home)}
            {railButton("plugins", "Плагины", Puzzle)}
            {sections.map((entry) => (
              <React.Fragment key={entry.entryId}>
                {sectionSlot("rail", entry.entryId)}
              </React.Fragment>
            ))}
            <div className="rail-bottom">
              <UiSlot
                slot="shell.rail.end"
                scope={workspaceScope}
                componentProps={slotProps}
              />
              <Button
                variant="ghost"
                size="icon"
                className="rail-button sidebar-toggle"
                title={
                  sidebarOpen
                    ? "Свернуть боковую панель"
                    : "Развернуть боковую панель"
                }
                aria-label={
                  sidebarOpen
                    ? "Свернуть боковую панель"
                    : "Развернуть боковую панель"
                }
                aria-expanded={sidebarOpen}
                aria-controls="section-sidebar"
                onClick={toggle}
              >
                {sidebarOpen ? <PanelLeftClose /> : <PanelLeftOpen />}
              </Button>
              {railButton("settings", "Настройки", SettingsIcon)}
            </div>
          </nav>
          <ResizablePanelGroup
            orientation="horizontal"
            resizeTargetMinimumSize={{ fine: 12, coarse: 24 }}
            className="shell-panels"
          >
            <ResizablePanel
              id="shell-sidebar-panel"
              panelRef={panel}
              defaultSize={initialPanelSize}
              collapsible
              collapsedSize={0}
              minSize={isMobile ? 0 : 192}
              maxSize={isMobile ? 0 : 480}
              groupResizeBehavior="preserve-pixel-size"
              onResize={(size) => {
                if (isMobile) return;
                const pixels = Math.round(size.inPixels);
                setCollapsed(pixels === 0);
                if (pixels >= 192) setWidth(clamp(pixels));
              }}
            >
              {!isMobile && (
                <Sidebar
                  id="section-sidebar"
                  collapsible="none"
                  className="sidebar section-sidebar"
                  aria-label="Боковая панель раздела"
                >
                  {sidebarContent}
                </Sidebar>
              )}
            </ResizablePanel>
            <ResizableHandle
              className={`sidebar-resize ${collapsed || isMobile ? "resize-hidden" : ""}`}
              aria-label="Ширина боковой панели"
              onDoubleClick={() => panel.current?.resize(272)}
            />
            <ResizablePanel id="shell-main-panel" minSize={isMobile ? 0 : 360}>
              <main className="main">
                <header className="topbar">
                  <div className="breadcrumbs">
                    <span>
                      {section === "chat"
                        ? "Чаты"
                        : section === "settings"
                          ? "Настройки"
                          : section === "plugins"
                            ? "Плагины"
                            : "Расширения"}
                    </span>
                    <span className="crumb-separator">/</span>
                    <strong>
                      {section === "chat"
                        ? session.title
                        : section === "settings"
                          ? ({
                              models: "Модели",
                              session: "Сессия",
                              mcp: "Серверы MCP",
                            }[tab] ?? "Параметры плагина")
                          : section === "plugins"
                            ? "Управление плагинами"
                            : sectionSlot("title", section)}
                    </strong>
                  </div>
                  <div className="shell-header-actions">
                    <UiSlot
                      slot="shell.header.actions"
                      scope={workspaceScope}
                      componentProps={slotProps}
                    />
                  </div>
                </header>
                <UiSlot
                  slot="shell.content.before"
                  scope={workspaceScope}
                  componentProps={slotProps}
                />
                <div className="chat-view" hidden={section !== "chat"}>
                  {children}
                </div>
                {section === "plugins" && <Plugins />}
                {section === "settings" && (
                  <section className="settings-screen" aria-label="Настройки">
                    {settings("body")}
                  </section>
                )}
                {!["chat", "plugins", "settings"].includes(section) &&
                  sectionSlot("body", section)}
                <UiSlot
                  slot="shell.content.after"
                  scope={workspaceScope}
                  componentProps={slotProps}
                />
              </main>
            </ResizablePanel>
          </ResizablePanelGroup>
          <Sheet open={isMobile && mobileOpen} onOpenChange={setMobileOpen}>
            <SheetContent
              side="left"
              className="oe-studio mobile-sidebar-sheet"
            >
              <SheetHeader className="oe-sr-only">
                <SheetTitle>Боковая панель</SheetTitle>
                <SheetDescription>Навигация активного раздела</SheetDescription>
              </SheetHeader>
              <Sidebar
                id="section-sidebar"
                collapsible="none"
                className="sidebar section-sidebar"
              >
                {sidebarContent}
              </Sidebar>
            </SheetContent>
          </Sheet>
        </div>
      </SidebarProvider>
    </TooltipProvider>
  );
}
