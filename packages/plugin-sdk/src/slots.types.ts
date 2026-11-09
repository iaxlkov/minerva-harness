export const slotDeclarations = {
  'root.layout': { kind: 'single', scope: 'root' },
  'shell.sections': { kind: 'keyed', scope: 'workspace' },
  'shell.rail.start': { kind: 'list', scope: 'workspace' },
  'shell.rail.end': { kind: 'list', scope: 'workspace' },
  'shell.sidebar.header': { kind: 'list', scope: 'workspace' },
  'shell.sidebar.content': { kind: 'list', scope: 'workspace' },
  'shell.sidebar.footer': { kind: 'list', scope: 'workspace' },
  'shell.header.actions': { kind: 'list', scope: 'workspace' },
  'shell.content.before': { kind: 'list', scope: 'workspace' },
  'shell.content.after': { kind: 'list', scope: 'workspace' },
  'navigation.items': { kind: 'list', scope: 'workspace' },
  'settings.pages': { kind: 'keyed', scope: 'workspace' },
  'conversation.views': { kind: 'keyed', scope: 'session' },
  'composer.actions': { kind: 'list', scope: 'session' },
  'composer.menu': { kind: 'list', scope: 'workspace' },
  'message.renderers': { kind: 'keyed', scope: 'session' },
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
export interface ShellSectionProps extends ShellNavigationProps {placement:'rail'|'sidebar'|'body'|'title';selected:boolean}
