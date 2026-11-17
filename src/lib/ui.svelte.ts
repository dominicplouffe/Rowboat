// Transient window state (not persisted).

import type { SectionId } from './types';

class Ui {
  settingsOpen = $state(false);
  settingsSection = $state<SectionId>('general');
  sidebarVisible = $state(true);
  historyOpen = $state(false);

  openSettings(section?: SectionId): void {
    if (section) this.settingsSection = section;
    this.settingsOpen = true;
  }
}

export const ui = new Ui();
