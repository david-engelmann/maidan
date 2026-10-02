// Types for the board check only. Not served. getElementById is used as the
// concrete control it returns, and a few errors carry a flag the page sets.

interface HTMLElement {
  value: string;
  checked: boolean;
  files: FileList | null;
  disabled: boolean;
  href: string;
  open: boolean;
  showModal(): void;
  close(): void;
}

interface Element {
  title: string;
  value: string;
  checked: boolean;
  files: FileList | null;
  disabled: boolean;
  href: string;
  open: boolean;
  dataset: DOMStringMap;
  tabIndex: number;
  onclick: ((this: GlobalEventHandlers, ev: MouseEvent) => unknown) | null;
  click(): void;
  focus(): void;
  showModal(): void;
  close(): void;
}

interface Event {
  key: string;
}

interface Error {
  said?: boolean;
}
