/**
 * Tiny zero-dep argv parser. Supports `--key=value`, `--key value`, and
 * boolean flags (`--flag`). Positional args are collected into `_`. Unknown
 * flags are kept in the returned map verbatim — command handlers validate.
 */
export type ParsedArgs = {
  _: string[];
  options: Record<string, string | boolean>;
};

export function parseArgv(argv: string[]): ParsedArgs {
  const _: string[] = [];
  const options: Record<string, string | boolean> = {};

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg.startsWith("--")) {
      _.push(arg);
      continue;
    }
    const eq = arg.indexOf("=");
    if (eq !== -1) {
      const key = arg.slice(2, eq);
      const value = arg.slice(eq + 1);
      options[key] = value;
      continue;
    }
    const key = arg.slice(2);
    const next = argv[i + 1];
    if (next !== undefined && !next.startsWith("--")) {
      options[key] = next;
      i++;
    } else {
      options[key] = true;
    }
  }

  return { _, options };
}

export function getString(args: ParsedArgs, key: string): string | undefined {
  const v = args.options[key];
  return typeof v === "string" ? v : undefined;
}

export function getBool(args: ParsedArgs, key: string): boolean {
  return args.options[key] === true || args.options[key] === "true";
}
