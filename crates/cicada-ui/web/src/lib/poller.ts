// Polling: the API has no push, so the console refreshes the open page every few seconds. The Live switch pauses it;
// Refresh fetches once. Refreshes never overlap: one asked for during another runs right after it. A hidden tab
// does not poll.

export const POLL_INTERVAL_MS = 5000;

export class Poller {
  paused = false;
  busy = false;
  lastAt: Date | null = null;
  error: unknown = null;
  private again = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly onVisible = () => {
    if (document.visibilityState === 'visible' && !this.paused) void this.refresh();
  };

  /** [tick] does one refresh (and may throw); [onChange] is called whenever the state above changes. */
  constructor(
    private readonly tick: () => Promise<void>,
    private readonly onChange: () => void,
    private readonly intervalMs: number = POLL_INTERVAL_MS,
  ) {
    document.addEventListener('visibilitychange', this.onVisible);
  }

  dispose(): void {
    clearTimeout(this.timer);
    document.removeEventListener('visibilitychange', this.onVisible);
  }

  setPaused(paused: boolean): void {
    this.paused = paused;
    if (paused) clearTimeout(this.timer);
    else void this.refresh();
    this.onChange();
  }

  /** Refreshes now (also while paused: that is the Refresh button). Resolves when this refresh is done. */
  async refresh(): Promise<void> {
    if (this.busy) {
      this.again = true;
      return;
    }
    clearTimeout(this.timer);
    this.busy = true;
    this.onChange();
    try {
      await this.tick();
      this.lastAt = new Date();
      this.error = null;
    } catch (e) {
      this.error = e;
    } finally {
      this.busy = false;
      this.onChange();
    }
    if (this.again) {
      this.again = false;
      return this.refresh();
    }
    this.schedule();
  }

  private schedule(): void {
    clearTimeout(this.timer);
    if (this.paused) return;
    this.timer = setTimeout(() => {
      if (document.visibilityState === 'hidden') this.schedule();
      else void this.refresh();
    }, this.intervalMs);
  }
}
