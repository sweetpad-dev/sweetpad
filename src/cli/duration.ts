// "30" / "30s" / "5m" / "1h" / "1h30m" / "2h15m10s" → seconds. Bare numbers = seconds.
export function parseDuration(input: string): number | undefined {
  const trimmed = input.trim();
  if (trimmed.length === 0) return undefined;
  if (/^\d+(\.\d+)?$/.test(trimmed)) {
    return Number.parseFloat(trimmed);
  }
  const re = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/;
  const match = re.exec(trimmed);
  if (!match) return undefined;
  const [, h, m, s] = match;
  if (h === undefined && m === undefined && s === undefined) return undefined;
  const hours = h ? Number.parseInt(h, 10) : 0;
  const mins = m ? Number.parseInt(m, 10) : 0;
  const secs = s ? Number.parseInt(s, 10) : 0;
  return hours * 3600 + mins * 60 + secs;
}
