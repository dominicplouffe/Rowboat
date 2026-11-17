// Modal prompts raised from anywhere in the app and answered by the user.

import type { ConnectionView } from './types';

export interface ConfirmRequest {
  title: string;
  message: string;
  /** Shown in a monospace block: the SQL, a fingerprint, … */
  detail?: string;
  confirmLabel: string;
  /** Styles the confirm button as destructive. */
  danger?: boolean;
  /** Accent for Prod connections. */
  prod?: boolean;
}

class Dialogs {
  confirmRequest = $state<(ConfirmRequest & { resolve: (ok: boolean) => void }) | null>(null);
  /** The connection being edited; `{}` for a new one; null when closed. */
  connection = $state<{ editing: ConnectionView | null } | null>(null);

  confirm(request: ConfirmRequest): Promise<boolean> {
    // Answer any prompt still open as "no" before replacing it.
    this.confirmRequest?.resolve(false);
    return new Promise((resolve) => {
      this.confirmRequest = { ...request, resolve };
    });
  }

  answer(ok: boolean): void {
    const request = this.confirmRequest;
    this.confirmRequest = null;
    request?.resolve(ok);
  }

  get anyOpen(): boolean {
    return this.confirmRequest !== null || this.connection !== null;
  }
}

export const dialogs = new Dialogs();
