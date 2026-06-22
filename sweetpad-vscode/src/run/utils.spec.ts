import type { TaskTerminal, TerminalWriteOptions } from "../common/tasks/types";
import { renderNdjsonLine, writeErrorLine } from "./utils";

// biome-ignore lint/suspicious/noControlCharactersInRegex: intentional — matching ANSI escape sequences
const ANSI_RE = /\x1b\[[0-9;]*m/g;

type CapturedLine = { raw: string; clean: string };

function createMockTerminal(): { terminal: TaskTerminal; lines: CapturedLine[] } {
  const lines: CapturedLine[] = [];
  let buffer = "";

  const terminal: TaskTerminal = {
    write(data: string, options?: TerminalWriteOptions): void {
      buffer += data;
      if (options?.newLine) {
        lines.push({ raw: buffer, clean: buffer.replace(ANSI_RE, "") });
        buffer = "";
      }
    },
    execute(): Promise<void> {
      throw new Error("execute() is not expected in unit tests");
    },
    runGroup(): Promise<never> {
      throw new Error("runGroup() is not expected in unit tests");
    },
  };

  return { terminal, lines };
}

describe("renderNdjsonLine", () => {
  it("renders a valid ndjson entry with a one-letter level and the category", () => {
    const { terminal, lines } = createMockTerminal();
    renderNdjsonLine(
      JSON.stringify({
        timestamp: "2026-04-19 21:47:53.979612-0700",
        messageType: "Notice",
        category: "App",
        eventMessage: "hello",
      }),
      terminal,
    );
    expect(lines).toHaveLength(1);
    expect(lines[0].clean).toBe("21:47:53.979 N [App] hello");
  });

  it("maps every messageType to the expected letter", () => {
    const cases: Array<[string, string]> = [
      ["Debug", "D"],
      ["Info", "I"],
      ["Default", "N"],
      ["Notice", "N"],
      ["Error", "E"],
      ["Fault", "F"],
    ];
    for (const [messageType, expectedLetter] of cases) {
      const { terminal, lines } = createMockTerminal();
      renderNdjsonLine(
        JSON.stringify({
          timestamp: "2026-04-19 21:47:53.979612-0700",
          messageType,
          category: "App",
          eventMessage: "x",
        }),
        terminal,
      );
      expect(lines[0].clean).toBe(`21:47:53.979 ${expectedLetter} [App] x`);
    }
  });

  it("falls back to '?' letter and 'gray' color for unknown messageType", () => {
    const { terminal, lines } = createMockTerminal();
    renderNdjsonLine(
      JSON.stringify({
        timestamp: "2026-04-19 21:47:53.979612-0700",
        messageType: "Weird",
        category: "cat",
        eventMessage: "msg",
      }),
      terminal,
    );
    expect(lines[0].clean).toBe("21:47:53.979 ? [cat] msg");
  });

  it("renders non-JSON lines as info-level [system] banners", () => {
    const { terminal, lines } = createMockTerminal();
    renderNdjsonLine("Filtering the log data using ...", terminal);
    expect(lines).toHaveLength(1);
    expect(lines[0].clean).toMatch(/^\d{2}:\d{2}:\d{2}\.\d{3} N \[system\] Filtering the log data using \.\.\.$/);
  });
});

describe("writeErrorLine", () => {
  it("renders [system] error", () => {
    const { terminal, lines } = createMockTerminal();
    writeErrorLine(terminal, "system", "getpwuid_r failed");
    expect(lines).toHaveLength(1);
    expect(lines[0].clean).toMatch(/^\d{2}:\d{2}:\d{2}\.\d{3} E \[system\] getpwuid_r failed$/);
  });

  it("renders [pymobiledevice3] error", () => {
    const { terminal, lines } = createMockTerminal();
    writeErrorLine(terminal, "pymobiledevice3", "usbmux connection refused");
    expect(lines).toHaveLength(1);
    expect(lines[0].clean).toMatch(/^\d{2}:\d{2}:\d{2}\.\d{3} E \[pymobiledevice3\] usbmux connection refused$/);
  });
});
