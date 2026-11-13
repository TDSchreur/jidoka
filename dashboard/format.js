// Pure formatters for the dashboard: times, durations, tokens and cost. No DOM.

export const timestamp = (run) =>
  typeof run.updated_at === 'number' &&
  Number.isFinite(run.updated_at) &&
  !Number.isNaN(new Date(run.updated_at).getTime())
    ? run.updated_at
    : null;

/** Fresh tokens: input excluding cache reads, plus cache writes and output; null when neither input nor output is known. */
export const freshTokens = (t) => (t.input == null && t.output == null ? null : (t.input ?? 0) + (t.cache_write ?? 0) + (t.output ?? 0));

/** Fresh tokens over all rows of a Run's usage, or null when nothing is tracked. Unknown is never 0. */
export function usageTotal(run) {
  const usage = run.usage;
  if (!usage || usage.status === 'not-tracked' || !Array.isArray(usage.rows)) return null;
  const counts = usage.rows.map(freshTokens).filter((n) => n !== null);
  return counts.length ? counts.reduce((a, b) => a + b, 0) : null;
}

/** Dollars as text: `$1.23`, `<$0.01`, or `unknown` (never $0) for null. */
export function usdText(usd) {
  if (usd === null || usd === undefined) return 'unknown';
  return usd > 0 && usd < 0.005 ? '<$0.01' : `$${usd.toFixed(2)}`;
}

/** An estimated cost from the server: `$1.23`, `unknown`, or `>= $1.23 (partial)`. For a total, `withModels` also names the unpriced models. */
export function costText(cost, withModels = false) {
  const text = cost?.partial && cost.usd != null ? `>= ${usdText(cost.usd)} (partial)` : usdText(cost?.usd);
  const models = withModels ? cost?.unpriced_models ?? [] : [];
  return models.length ? `${text}, ${models.length} unpriced: ${models.join(', ')}` : text;
}

export const compactTokens = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n));
export const tokenCount = (n) => (n === undefined || n === null ? 'n/a' : n.toLocaleString('en-US'));

/** "just now", "5m ago", "3h ago", "2d ago". */
export function ago(ms) {
  const minutes = Math.floor(Math.max(0, ms) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/**
 * A time in the Run sheet: "09:22" for today, "Yesterday 16:40", otherwise weekday, day, month and time, such as
 * "Fri 6 Nov 14:02". Day and month names follow the browser's locale; `now` is passed in so the result is repeatable.
 */
export function sheetTime(ms, now = Date.now()) {
  const date = new Date(ms);
  const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  const startOfDay = (value) => new Date(value).setHours(0, 0, 0, 0);
  const days = Math.round((startOfDay(now) - startOfDay(ms)) / 86400000);
  if (days === 0) return time;
  if (days === 1) return `Yesterday ${time}`;
  const day = date.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' }).replace(',', '');
  return `${day} ${time}`;
}


/** A duration in plain words: "under 1 min", "12 min", "1 h 5 min". */
export function durationWords(ms) {
  const minutes = Math.floor(ms / 60000);
  if (!(minutes >= 1)) return 'under 1 min';
  if (minutes < 60) return `${minutes} min`;
  const rest = minutes % 60;
  return rest ? `${Math.floor(minutes / 60)} h ${rest} min` : `${Math.floor(minutes / 60)} h`;
}

