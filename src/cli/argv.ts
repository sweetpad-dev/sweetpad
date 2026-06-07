/**
 * Tiny argv parser tailored to the sweetpad CLI shape:
 *
 *   sweetpad <method.name> [positional...] [--flag value | --flag=value]
 *
 * The first non-flag token is the full RPC method name (must contain a `.`); a
 * bare word leaves `method` undefined and the dispatcher prints usage. All
 * subsequent non-flag tokens go straight into `positionals`. Reserved flags:
 * --raw, --help / -h.
 */

export type ParsedArgv = {
  /** Full method name (e.g. "scheme.list", "buildSettings.get"). */
  method: string | undefined;
  positionals: string[];
  flags: Record<string, string | boolean>;
  raw: boolean;
  help: boolean;
};

export function parseArgv(argv: string[]): ParsedArgv {
  const result: ParsedArgv = {
    method: undefined,
    positionals: [],
    flags: {},
    raw: false,
    help: false,
  };

  let i = 0;
  let firstSeen = false;
  while (i < argv.length) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      result.help = true;
      i += 1;
      continue;
    }
    if (arg === "--raw") {
      result.raw = true;
      i += 1;
      continue;
    }
    if (arg.startsWith("--")) {
      const eqIdx = arg.indexOf("=");
      let key: string;
      let value: string | boolean;
      if (eqIdx >= 0) {
        key = arg.slice(2, eqIdx);
        value = arg.slice(eqIdx + 1);
      } else {
        key = arg.slice(2);
        const next = argv[i + 1];
        if (next !== undefined && !next.startsWith("-")) {
          value = next;
          i += 1;
        } else {
          value = true;
        }
      }
      result.flags[key] = value;
      i += 1;
      continue;
    }
    // First non-flag token is the dotted method name; a bare word leaves
    // `method` undefined (→ usage). Everything after is a positional.
    if (!firstSeen) {
      firstSeen = true;
      if (arg.includes(".")) result.method = arg;
      i += 1;
      continue;
    }
    result.positionals.push(arg);
    i += 1;
  }
  return result;
}
