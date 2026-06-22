import type {
  ProcessGroup,
  ProcessHandle,
  ProcessSpec,
  TaskTerminal,
  TerminalWriteOptions,
} from "../common/tasks/types";
import { MainExecutable } from "./main";

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

function createMockGroup(terminal: TaskTerminal): ProcessGroup {
  const handle: ProcessHandle = {
    pid: 1,
    exit: new Promise(() => {}),
    kill: () => {},
    onData: () => {},
    onError: () => {},
  };
  return {
    terminal,
    spawn: (_spec: ProcessSpec) => handle,
  };
}

function createMain(terminal: TaskTerminal): MainExecutable {
  return new MainExecutable(createMockGroup(terminal), { command: "fake" });
}

describe("MainExecutable.processStderrLine", () => {
  it("re-renders an Apple-format stderr line with the captured time and category", () => {
    const { terminal, lines } = createMockTerminal();
    createMain(terminal).processStderrLine("2026-04-19 21:47:53.979 Laboratory[1234:abc] [App] hello");
    expect(lines).toHaveLength(1);
    expect(lines[0].clean).toBe("21:47:53.979 N [App] hello");
  });

  it("treats unparseable output as a [print] line", () => {
    const { terminal, lines } = createMockTerminal();
    createMain(terminal).processStderrLine("just some plain output");
    expect(lines).toHaveLength(1);
    expect(lines[0].clean).toMatch(/^\d{2}:\d{2}:\d{2}\.\d{3} N \[print\] just some plain output$/);
  });
});
