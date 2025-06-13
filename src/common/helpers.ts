export function uniqueFilter<T>(value: T, index: number, array: T[]): boolean {
  return array.indexOf(value) === index;
}

type ConfigEnv = { [key: string]: string | null };
type PreparedEnv = { [key: string]: string | undefined };

/**
 * Usually, we get from config object a object with string values or null, but to pass it to execa
 * we need to convert "null" values to "undefined"
 */
export function prepareEnvVars(env: ConfigEnv | undefined): PreparedEnv {
  if (env === undefined) {
    return {};
  }

  const result: Record<string, string | undefined> = {};
  for (const [key, value] of Object.entries(env)) {
    result[key] = value ?? undefined;
  }
  return result;
}

/**
 * Expands environment variables in a string
 * Example: "Hello ${env:USER}" -> "Hello john"
 */
export function expandEnvVars(value: string): string {
  return value.replace(/\${env:([^}]+)}/g, (match, name) => {
    return process.env[name] ?? match;
  });
}
