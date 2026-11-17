// Lets actions outside the editor component (Run, shortcuts) reach the live
// editor without importing CodeMirror into the startup bundle.

export interface EditorHandle {
  selection(): { from: number; to: number };
  markError(offset: number | null): void;
  focus(): void;
  text(): string;
  replace(insert: string, from?: number, to?: number): void;
}

let current: EditorHandle | null = null;

export const editorBridge = {
  attach(handle: EditorHandle | null): void {
    current = handle;
  },
  selection: () => current?.selection() ?? { from: 0, to: 0 },
  markError: (offset: number | null) => current?.markError(offset),
  focus: () => current?.focus(),
  text: () => current?.text() ?? '',
  replace: (insert: string, from?: number, to?: number) => current?.replace(insert, from, to),
};
