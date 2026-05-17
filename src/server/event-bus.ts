import type { WireEvent } from "../protocol/types";

export type EventSubscriber = (event: WireEvent) => void;

/**
 * In-memory pub/sub keyed by buildId. The build method emits; the attach
 * handler subscribes. Subscribers are notified synchronously — order
 * matters (so a client gets `build.started` before any `log.line`).
 *
 * Cleared subscribers stop receiving immediately. Emit-after-unsubscribe is
 * a no-op.
 */
export class EventBus {
  private readonly subscribers = new Map<string, Set<EventSubscriber>>();

  subscribe(buildId: string, callback: EventSubscriber): () => void {
    let set = this.subscribers.get(buildId);
    if (!set) {
      set = new Set();
      this.subscribers.set(buildId, set);
    }
    set.add(callback);

    return () => {
      const current = this.subscribers.get(buildId);
      if (!current) return;
      current.delete(callback);
      if (current.size === 0) this.subscribers.delete(buildId);
    };
  }

  emit(buildId: string, event: WireEvent): void {
    const set = this.subscribers.get(buildId);
    if (!set) return;
    // Iterate over a snapshot — a subscriber that unsubscribes while being
    // notified would mutate the set otherwise.
    for (const cb of [...set]) {
      try {
        cb(event);
      } catch {
        // Subscriber callbacks must not break peer subscribers. Swallow.
      }
    }
  }

  subscriberCount(buildId: string): number {
    return this.subscribers.get(buildId)?.size ?? 0;
  }
}
