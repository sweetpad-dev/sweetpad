/**
 * Tiny argv parser tailored to the sweetpad CLI shape:
 *
 *   sweetpad <method.name> [positional...] [--flag value | --flag=value]
 *
 * The first non-flag token is the full RPC method name (must contain a `.`).
 * All subsequent non-flag tokens go straight into `positionals`. Reserved
 * flags: --server, --raw, --help / -h.
 */

export type ParsedArgv = {
  /** Full method name (e.g. "scheme.list", "buildSettings.get"). */
  method: string | undefined;
  /**
   * Special non-RPC subcommand (e.g. `servers`) when the first token is a bare
   * word with no dot. Populated mutually exclusively with `method`.
   */
  subcommand: string | undefined;
  /** Subcommand action — only set when `subcommand` is. */
  subcommandAction: string | undefined;
  positionals: string[];
  flags: Record<string, string | boolean>;
  server: string | undefined;
  raw: boolean;
  help: boolean;
};

export function parseArgv(argv: string[]): ParsedArgv {
  const result: ParsedArgv = {
    method: undefined,
    subcommand: undefined,
    subcommandAction: undefined,
    positionals: [],
    flags: {},
    server: undefined,
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
      if (key === "server") {
        result.server = typeof value === "string" ? value : undefined;
      } else {
        result.flags[key] = value;
      }
      i += 1;
      continue;
    }
    // First positional: either a dotted method name or a bare subcommand word.
    if (!firstSeen) {
      firstSeen = true;
      if (arg.includes(".")) {
        result.method = arg;
      } else {
        result.subcommand = arg;
      }
      i += 1;
      continue;
    }
    // Second positional: when we have a subcommand it doubles as the action;
    // otherwise it lands in `positionals` like every other arg.
    if (result.subcommand && result.subcommandAction === undefined) {
      result.subcommandAction = arg;
    } else {
      result.positionals.push(arg);
    }
    i += 1;
  }
  return result;
}
