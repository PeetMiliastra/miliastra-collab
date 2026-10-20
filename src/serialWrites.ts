import { useRef } from "react";
/** Runs writes in order. A failed write doesn't stop later ones; flush reports the last error. */
export class SerialWrites {
  private pending: Promise<unknown> = Promise.resolve();
  enqueue<T>(write: () => Promise<T>): Promise<T> {
    const next = this.pending.catch(() => {}).then(write);
    this.pending = next;
    return next;
  }
  flush(): Promise<unknown> {
    return this.pending;
  }
}
export function useSerialWrites() {
  const queue = useRef<SerialWrites | null>(null);
  if (!queue.current) queue.current = new SerialWrites();
  return queue.current;
}
