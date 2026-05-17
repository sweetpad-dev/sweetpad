import { describe, expect, it, vi } from "vitest";

import { MessageFramer, encodeMessage } from "./framing";
import type { WireMessage, WireRequest } from "./types";

describe("MessageFramer", () => {
  it("parses one message per newline", () => {
    const messages: WireMessage[] = [];
    const framer = new MessageFramer({ onMessage: (m) => messages.push(m) });
    framer.append('{"id":1,"method":"build"}\n');
    framer.append('{"id":2,"method":"clean"}\n');
    expect(messages).toHaveLength(2);
    expect((messages[0] as WireRequest).method).toBe("build");
    expect((messages[1] as WireRequest).method).toBe("clean");
  });

  it("handles a message split across two appends", () => {
    const messages: WireMessage[] = [];
    const framer = new MessageFramer({ onMessage: (m) => messages.push(m) });
    framer.append('{"id":1,"meth');
    expect(messages).toHaveLength(0);
    framer.append('od":"build"}\n');
    expect(messages).toHaveLength(1);
  });

  it("reports malformed lines via onError without throwing", () => {
    const onError = vi.fn();
    const framer = new MessageFramer({ onMessage: vi.fn(), onError });
    framer.append("not-json\n");
    expect(onError).toHaveBeenCalledOnce();
    expect(onError.mock.calls[0][0]).toBe("not-json");
  });

  it("encodeMessage produces a single newline-terminated JSON line", () => {
    const encoded = encodeMessage({ id: 1, method: "build", params: {} });
    expect(encoded.endsWith("\n")).toBe(true);
    expect(encoded.split("\n").length).toBe(2);
    expect(JSON.parse(encoded.trim())).toEqual({ id: 1, method: "build", params: {} });
  });
});
