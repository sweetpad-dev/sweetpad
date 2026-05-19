// Shebang is injected at bundle time by rolldown's `output.banner` — leaving
// it out of source keeps tsc / vitest / tsx happy.

import { parseArgv } from "./argv";
import { dispatchCli } from "./commands";

function write(out: NodeJS.WriteStream, value: unknown, raw: boolean): void {
  if (value === undefined) return;
  const text = typeof value === "string" ? value : raw ? JSON.stringify(value) : JSON.stringify(value, null, 2);
  out.write(`${text}\n`);
}

async function main(): Promise<void> {
  const parsed = parseArgv(process.argv.slice(2));
  const exit = await dispatchCli(parsed);

  if (exit.stdout !== undefined) write(process.stdout, exit.stdout, parsed.raw);
  if (exit.stderr !== undefined) write(process.stderr, exit.stderr, parsed.raw);
  process.exit(exit.code);
}

main().catch((err) => {
  const message = err instanceof Error ? err.message : String(err);
  process.stderr.write(`${JSON.stringify({ ok: false, error: { code: "CLI_CRASH", message } })}\n`);
  process.exit(2);
});
