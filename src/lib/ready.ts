import { ipc } from './ipc';

let reported = false;

/**
 * Tells the core, once, that the window is usable (the editor accepts input).
 * The core only acts on this when it is being benchmarked.
 *
 * Waits for the next paint so the measurement includes rendering, but never
 * indefinitely: webviews stop firing animation frames for hidden or occluded
 * windows.
 */
export function reportReady(): void {
  if (reported) return;
  reported = true;
  let sent = false;
  const send = () => {
    if (sent) return;
    sent = true;
    void ipc.appReady().catch(() => {});
  };
  requestAnimationFrame(send);
  setTimeout(send, 250);
}
