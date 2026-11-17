// Open editor tabs. Each tab keeps its own text and row limit.

export interface Tab {
  id: number;
  title: string;
  text: string;
  /** Rows per fetch for this tab; 0 means no limit. */
  rowLimit: number;
}

class Tabs {
  list = $state<Tab[]>([]);
  activeId = $state(0);
  #nextId = 1;

  get active(): Tab | undefined {
    return this.list.find((tab) => tab.id === this.activeId);
  }

  open(rowLimit: number, text = ''): Tab {
    const id = this.#nextId++;
    this.list.push({ id, title: `Query ${id}`, text, rowLimit });
    this.activeId = id;
    // Return the reactive proxy stored in the list, not the plain object.
    return this.list[this.list.length - 1];
  }

  close(id: number): void {
    const index = this.list.findIndex((tab) => tab.id === id);
    if (index === -1) return;
    this.list.splice(index, 1);
    if (this.activeId === id) {
      this.activeId = this.list[Math.min(index, this.list.length - 1)]?.id ?? 0;
    }
  }

  /** Moves the selection by `delta` tabs, wrapping around. */
  cycle(delta: number): void {
    if (this.list.length === 0) return;
    const index = this.list.findIndex((tab) => tab.id === this.activeId);
    const next = (index + delta + this.list.length) % this.list.length;
    this.activeId = this.list[next].id;
  }
}

export const tabs = new Tabs();
