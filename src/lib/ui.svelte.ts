// Transient window state (not persisted).

import { ipc } from './ipc';
import type { SectionId, UpdateInfo } from './types';

class Ui {
  settingsOpen = $state(false);
  settingsSection = $state<SectionId>('general');
  sidebarVisible = $state(true);
  historyOpen = $state(false);
  /** A newer release, once a check has found one. */
  update = $state<UpdateInfo | null>(null);

  /**
   * Looks for a newer release. Automatic checks (`manual` false) do nothing
   * when update checks are off and stay quiet on failure. Returns the error
   * message of a failed manual check, or null.
   */
  async checkForUpdate(manual: boolean): Promise<string | null> {
    try {
      this.update = await ipc.updateCheck(manual);
      return null;
    } catch (e) {
      return manual ? String(e) : null;
    }
  }

  openSettings(section?: SectionId): void {
    if (section) this.settingsSection = section;
    this.settingsOpen = true;
  }
}

export const ui = new Ui();
