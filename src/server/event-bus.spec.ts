import { describe, expect, it, vi } from "vitest";

import type { WireEvent } from "../protocol/types";
import { EventBus } from "./event-bus";

function makeEvent(buildId: string, payload = "hi"): WireEvent {
  return {
    event: "log.line",
    schemaVersion: "1.0",
    ts: "2026-01-01T00:00:00.000Z",
    buildId,
    data: { line: payload },
  };
}

describe("EventBus", () => {
  it("delivers events to every subscriber for the matching buildId", () => {
    const bus = new EventBus();
    const a = vi.fn();
    const b = vi.fn();
    bus.subscribe("b1", a);
    bus.subscribe("b1", b);

    bus.emit("b1", makeEvent("b1"));

    expect(a).toHaveBeenCalledOnce();
    expect(b).toHaveBeenCalledOnce();
  });

  it("does not deliver events to subscribers of other builds", () => {
    const bus = new EventBus();
    const a = vi.fn();
    bus.subscribe("b1", a);
    bus.emit("b2", makeEvent("b2"));
    expect(a).not.toHaveBeenCalled();
  });

  it("subscriberCount drops when unsubscribed", () => {
    const bus = new EventBus();
    const unsub = bus.subscribe("b1", () => {});
    expect(bus.subscriberCount("b1")).toBe(1);
    unsub();
    expect(bus.subscriberCount("b1")).toBe(0);
  });

  it("emit-after-unsubscribe is a no-op", () => {
    const bus = new EventBus();
    const cb = vi.fn();
    const unsub = bus.subscribe("b1", cb);
    unsub();
    bus.emit("b1", makeEvent("b1"));
    expect(cb).not.toHaveBeenCalled();
  });

  it("survives a subscriber throwing", () => {
    const bus = new EventBus();
    const bad = vi.fn(() => {
      throw new Error("kaboom");
    });
    const good = vi.fn();
    bus.subscribe("b1", bad);
    bus.subscribe("b1", good);

    expect(() => bus.emit("b1", makeEvent("b1"))).not.toThrow();
    expect(good).toHaveBeenCalled();
  });

  it("tolerates a subscriber unsubscribing during notify", () => {
    const bus = new EventBus();
    const a = vi.fn();
    const b = vi.fn();
    const unsubA = bus.subscribe("b1", () => {
      a();
      unsubA();
    });
    bus.subscribe("b1", b);

    bus.emit("b1", makeEvent("b1"));
    expect(a).toHaveBeenCalledOnce();
    expect(b).toHaveBeenCalledOnce();
  });
});
