export const slotDeclarations = {
  'root.layout': { kind: 'single', scope: 'root' },
  'navigation.items': { kind: 'list', scope: 'workspace' },
  'settings.pages': { kind: 'keyed', scope: 'workspace' },
  'conversation.views': { kind: 'keyed', scope: 'session' },
  'composer.actions': { kind: 'list', scope: 'session' },
  'message.renderers': { kind: 'keyed', scope: 'session' },
  'tool.renderers': { kind: 'keyed', scope: 'session' },
  'workspace.panels': { kind: 'list', scope: 'workspace' },
} as const;
export type SlotDeclarations = typeof slotDeclarations;
export type SlotName = keyof SlotDeclarations;
// React-free contract; the renderer adapter validates the component boundary.
export type SlotComponent = (props: any) => any;
export interface SlotEntry { ownerPluginId: string; entryId: string; component: SlotComponent; order?: number }
export type SlotSelection = Partial<Record<SlotName, string | Record<string, string>>>;
