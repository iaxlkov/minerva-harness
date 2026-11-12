export const slotDeclarations = {
  'root.layout': { kind: 'single', scope: 'root' },
  'shell.sections': { kind: 'keyed', scope: 'workspace' },
  'shell.rail.start': { kind: 'list', scope: 'workspace' },
  'shell.rail.end': { kind: 'list', scope: 'workspace' },
  'shell.sidebar.header': { kind: 'list', scope: 'workspace' },
  'shell.sidebar.content': { kind: 'list', scope: 'workspace' },
  'shell.sidebar.footer': { kind: 'list', scope: 'workspace' },
  'shell.header.actions': { kind: 'list', scope: 'workspace' },
  'shell.titlebar.start': { kind: 'list', scope: 'workspace' },
  'shell.titlebar.end': { kind: 'list', scope: 'workspace' },
  'shell.titlebar.menu': { kind: 'list', scope: 'workspace' },
  'shell.canvas.tabs': { kind: 'keyed', scope: 'workspace' },
  'shell.canvas.actions': { kind: 'list', scope: 'workspace' },
  'shell.content.before': { kind: 'list', scope: 'workspace' },
  'shell.content.after': { kind: 'list', scope: 'workspace' },
  'editor.tabs.start': { kind: 'list', scope: 'workspace' },
  'editor.tabs.end': { kind: 'list', scope: 'workspace' },
  'navigation.items': { kind: 'list', scope: 'workspace' },
  'settings.pages': { kind: 'keyed', scope: 'workspace' },
  'conversation.views': { kind: 'keyed', scope: 'session' },
  'session.controls': { kind: 'list', scope: 'workspace' },
  'composer.access': { kind: 'list', scope: 'workspace' },
  'tool.interactions': { kind: 'list', scope: 'workspace' },
  'composer.queue': { kind: 'list', scope: 'workspace' },
  'composer.actions': { kind: 'list', scope: 'session' },
  'composer.menu': { kind: 'list', scope: 'workspace' },
  'message.renderers': { kind: 'keyed', scope: 'session' },
  'message.actions': { kind: 'list', scope: 'session' },
  'message.code': { kind: 'keyed', scope: 'workspace' },
  'composer.model.menu': { kind: 'list', scope: 'session' },
  'composer.footer': { kind: 'list', scope: 'session' },
  'composer.image.tools': { kind: 'list', scope: 'workspace' },
  'tool.renderers': { kind: 'keyed', scope: 'session' },
  'workspace.panels': { kind: 'list', scope: 'workspace' },
  'workspace.menu': { kind: 'list', scope: 'workspace' },
  'contextMenu.icon': { kind: 'keyed', scope: 'workspace' },
} as const;
export type SlotDeclarations = typeof slotDeclarations;
export type SlotName = keyof SlotDeclarations;
// React-free contract; the renderer adapter validates the component boundary.
export type SlotComponent = (props: any) => any;
export interface SlotEntry { ownerPluginId: string; entryId: string; component: SlotComponent; order?: number }
export type SlotSelection = Partial<Record<SlotName, string | Record<string, string>>>;

/** Props supplied by Minerva to workspace-scoped shell contributions. */
export interface ShellNavigationProps {section:string;onNavigate:(section:string)=>void}
export interface ShellSectionProps extends ShellNavigationProps {sessionId:string;placement:'rail'|'sidebar'|'body'|'title';selected:boolean}
export interface EditorTabsSlotProps {sessionId:string;activePath:string|null}
