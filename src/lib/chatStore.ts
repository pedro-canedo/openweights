// Estado compartilhado das conversas: a sidebar do App e a tela de Chat
// leem/escrevem aqui (useSyncExternalStore).

import type { ChatRow } from "./types";

export interface ChatSnapshot {
  chats: ChatRow[];
  activeId: number | null;
  /** Abrir esta conversa quando o Chat montar / estiver visível. */
  openId: number | null;
  /** Começar conversa em branco. */
  openNew: boolean;
}

let snap: ChatSnapshot = {
  chats: [],
  activeId: null,
  openId: null,
  openNew: false,
};

const listeners = new Set<() => void>();

function emit(next: ChatSnapshot): void {
  snap = next;
  for (const fn of listeners) fn();
}

export const chatStore = {
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },
  get(): ChatSnapshot {
    return snap;
  },
  setChats(chats: ChatRow[]): void {
    emit({ ...snap, chats });
  },
  setActive(id: number | null): void {
    if (snap.activeId === id) return;
    emit({ ...snap, activeId: id });
  },
  requestOpen(id: number): void {
    emit({ ...snap, openId: id, openNew: false, activeId: id });
  },
  requestNew(): void {
    emit({ ...snap, openNew: true, openId: null, activeId: null });
  },
  clearPending(): void {
    if (snap.openId == null && !snap.openNew) return;
    emit({ ...snap, openId: null, openNew: false });
  },
};
