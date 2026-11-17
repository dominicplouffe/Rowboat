// Every keyboard-reachable action in the app. Shown in Settings → Keyboard.

import { settings } from './settings.svelte';
import type { Action } from './shortcuts';
import { tabs } from './tabs.svelte';
import { ui } from './ui.svelte';

const newTab = () => tabs.open(settings.value?.execution.defaultRowLimit ?? 1000);

export const ACTIONS: Action[] = [
  {
    id: 'tab.new',
    group: 'Tabs',
    label: 'New query tab',
    chord: 'Mod+T',
    run: () => void newTab(),
  },
  {
    id: 'tab.close',
    group: 'Tabs',
    label: 'Close tab',
    chord: 'Mod+W',
    run: () => tabs.close(tabs.activeId),
  },
  {
    id: 'tab.next',
    group: 'Tabs',
    label: 'Next tab',
    chord: 'Mod+Alt+ArrowRight',
    run: () => tabs.cycle(1),
  },
  {
    id: 'tab.previous',
    group: 'Tabs',
    label: 'Previous tab',
    chord: 'Mod+Alt+ArrowLeft',
    run: () => tabs.cycle(-1),
  },
  {
    id: 'sidebar.toggle',
    group: 'Window',
    label: 'Show or hide the sidebar',
    chord: 'Mod+B',
    run: () => (ui.sidebarVisible = !ui.sidebarVisible),
  },
  {
    id: 'settings.open',
    group: 'Window',
    label: 'Open Settings',
    chord: 'Mod+,',
    run: () => ui.openSettings(),
  },
];
