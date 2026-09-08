import { useSyncExternalStore } from "react";

const revisions = new Map<string, number>();
const listeners = new Set<() => void>();
export function notifyProfileApplied(model: string) {
  revisions.set(model, (revisions.get(model) ?? 0) + 1);
  listeners.forEach(listener => listener());
}
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
export function useProfileRevision(model: string) {
  return useSyncExternalStore(subscribe, () => revisions.get(model) ?? 0);
}
