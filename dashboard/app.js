import { createContext, h, render } from 'preact';
import { useContext, useEffect, useRef, useState } from 'preact/hooks';
import htm from 'htm';
import { Help } from '/assets/help.js';

const html = htm.bind(h);

const GROUPS = [
  ['define', 'Define'],
  ['build', 'Build'],
  ['check', 'Check'],
  ['finish', 'Finish'],
];
const STATUSES = ['running', 'blocked', 'done'];
// A finished Run is called Shipped; its status stays `done` in the CLI, the API and runs.json.
const STATUS_LABEL = { running: 'Running', blocked: 'Blocked', done: 'Shipped' };
/** The status in sentence case; an unknown status shows as recorded. */
const statusLabel = (status) => STATUS_LABEL[status] ?? String(status);
const TICKET_STATUS = { pending: 'Pending', 'in-progress': 'In progress', done: 'Done' };
const REWORK_CAP = 3;
const POLL_MS = 3000;
const NOW_MS = 30000;
const THEMES = [
  ['system', 'System'],
  ['light', 'Light'],
  ['dark', 'Dark'],
];

const folders = (path) => String(path).split(/[\\/]+/).filter(Boolean);

/**
 * Maps each distinct Project path to a short name: its folder name, or "parent/folder" when two Projects
 * share a folder name. Linked worktrees report their own path, so they show as separate Projects.
 */
function projectNames(runs) {
  const paths = [...new Set(runs.map(projectOf).filter((path) => typeof path === 'string'))];
  const last = (path) => folders(path).at(-1) ?? path;
  const counts = new Map();
  for (const path of paths) counts.set(last(path), (counts.get(last(path)) ?? 0) + 1);
  return new Map(
    paths.map((path) => {
      const parts = folders(path);
      const name = counts.get(last(path)) > 1 && parts.length > 1 ? parts.slice(-2).join('/') : last(path);
      return [path, name];
    }),
  );
}

/** Projects hidden in this browser. */
function storedHiddenProjects() {
  try {
    const value = JSON.parse(localStorage.getItem('hidden-projects'));
    return new Set(Array.isArray(value) ? value.filter((path) => typeof path === 'string') : []);
  } catch {
    return new Set();
  }
}

/** The Project the page was opened on (`?project=<path>`), or null. */
function requestedProject() {
  try {
    return new URLSearchParams(location.search).get('project');
  } catch {
    return null;
  }
}

const ProjectNames = createContext(new Map());

/** The Project's name, with the full path as a tooltip. */
function ProjectName({ path }) {
  const name = useContext(ProjectNames).get(path) ?? path;
  return html`<span title=${path}>${name}</span>`;
}

const timestamp = (run) =>
  typeof run.updated_at === 'number' &&
  Number.isFinite(run.updated_at) &&
  !Number.isNaN(new Date(run.updated_at).getTime())
    ? run.updated_at
    : null;
/** Identifies a Run across Projects. Ids are global today; the Project keeps two rows from ever clashing. */
/** The Project a Run belongs to: its repository, so linked worktrees share a row, else where it was started. */
const projectOf = (run) => run.repository ?? run.project;
const runKey = (run) => `${projectOf(run)}\n${run.id}`;

// Blocked first, then the most recently updated; Runs without a timestamp go last, newest record first.
function compare(a, b) {
  const blocked = (b.run.status === 'blocked') - (a.run.status === 'blocked');
  if (blocked) return blocked;
  const ta = timestamp(a.run);
  const tb = timestamp(b.run);
  if (ta !== null && tb !== null && ta !== tb) return tb - ta;
  if ((ta === null) !== (tb === null)) return ta === null ? 1 : -1;
  return b.index - a.index;
}

function storedTheme() {
  try {
    const stored = localStorage.getItem('theme');
    if (stored === 'light' || stored === 'dark') return stored;
  } catch {
    /* storage unavailable: follow the system setting */
  }
  return 'system';
}

function applyTheme(choice) {
  if (choice === 'light' || choice === 'dark') document.documentElement.dataset.theme = choice;
  else delete document.documentElement.dataset.theme;
}

function ThemeSwitch() {
  const [theme, setTheme] = useState(storedTheme);
  const choose = (choice) => {
    setTheme(choice);
    applyTheme(choice);
    try {
      if (choice === 'system') localStorage.removeItem('theme');
      else localStorage.setItem('theme', choice);
    } catch {
      /* the choice still applies for this visit */
    }
  };
  return html`<fieldset class="theme">
    <legend>Theme</legend>
    ${THEMES.map(
      ([value, label]) => html`<label>
        <input
          type="radio"
          name="theme"
          value=${value}
          checked=${theme === value}
          onChange=${() => choose(value)}
        />
        ${' ' + label}
      </label>`,
    )}
  </fieldset>`;
}

/** The value as a URL when it is an absolute http(s) URL, else null. Only these become links. */
function httpUrl(value) {
  try {
    const url = new URL(String(value));
    return url.protocol === 'http:' || url.protocol === 'https:' ? url : null;
  } catch {
    return null;
  }
}

/** The tickets listed on a Run, with how many are done; null when none are recorded. */
const ticketProgress = (run) => {
  const tickets = Array.isArray(run.tickets) ? run.tickets : [];
  return tickets.length ? { done: tickets.filter((t) => t.status === 'done').length, total: tickets.length } : null;
};

/** Whether a stored reference can open in the Reader: a local path to a Markdown file, which is all the server serves. */
const isReadable = (value) => !httpUrl(value) && /\.md$/i.test(String(value));

/** A link for a stored reference, only when it is an http(s) URL; nothing otherwise. */
function RefLink({ value, label }) {
  const url = httpUrl(value);
  return url ? html`<a href=${url.href} target="_blank" rel="noopener noreferrer">${label}</a>` : null;
}

/** A button styled as a link that opens a run's local document (a spec or ticket) in the Reader. */
function ReaderLink({ run, reference, children, label }) {
  const [open, setOpen] = useState(false);
  return html`<button type="button" class="link-button" aria-label=${label} onClick=${() => setOpen(true)}>${children}</button
    >${open ? html`<${Reader} run=${run} reference=${reference} onClose=${() => setOpen(false)} />` : null}`;
}

/** A run's local document in a modal dialog over the page, fetched on every open. Escape and Close close only the Reader. */
function Reader({ run, reference, onClose }) {
  const dialog = useRef(null);
  const [doc, setDoc] = useState(null);
  useEffect(() => {
    const element = dialog.current;
    if (!element.open) element.showModal();
    // Escape closes the topmost modal dialog, here this one, and fires `close`.
    const onNativeClose = () => onClose();
    element.addEventListener('close', onNativeClose);
    return () => element.removeEventListener('close', onNativeClose);
  }, []);
  useEffect(() => {
    let current = true;
    fetch(`api/runs/${encodeURIComponent(run.id)}/document?reference=${encodeURIComponent(reference)}`)
      .then(async (response) => {
        const body = await response.json().catch(() => ({}));
        if (response.ok) return { name: body.name, html: body.html };
        if (body.available === false) return { gone: body.path ?? reference };
        return { error: body.error ?? `Could not load the document (${response.status})` };
      })
      .catch((error) => ({ error: String(error.message ?? error) }))
      .then((result) => current && setDoc(result));
    return () => {
      current = false;
    };
  }, []);
  const heading = doc?.name ?? reference.split(/[\\/]/).pop();
  return html`<dialog
    ref=${dialog}
    class="reader"
    aria-labelledby="reader-title"
    onClick=${(event) => event.stopPropagation()}
  >
    <div class="reader-bar">
      <h2 id="reader-title">${heading}</h2>
      <button type="button" class="toggle reader-close" onClick=${() => dialog.current.close()}>Close</button>
    </div>
    ${doc === null
      ? html`<p class="reader-note" role="status">Loading…</p>`
      : doc.gone !== undefined
        ? html`<p class="reader-note">This document is no longer available (${doc.gone})</p>`
        : doc.error !== undefined
          ? html`<p class="reader-note">${doc.error}</p>`
          : // The server escapes raw HTML and keeps only http(s) links, so this Markdown is safe to inject.
            html`<div class="reader-doc" dangerouslySetInnerHTML=${{ __html: doc.html }}></div>`}
  </dialog>`;
}

/** Fresh tokens: input excluding cache reads, plus cache writes and output; null when neither input nor output is known. */
const freshTokens = (t) => (t.input == null && t.output == null ? null : (t.input ?? 0) + (t.cache_write ?? 0) + (t.output ?? 0));

/** Fresh tokens over all rows of a Run's usage, or null when nothing is tracked. Unknown is never 0. */
function usageTotal(run) {
  const usage = run.usage;
  if (!usage || usage.status === 'not-tracked' || !Array.isArray(usage.rows)) return null;
  const counts = usage.rows.map(freshTokens).filter((n) => n !== null);
  return counts.length ? counts.reduce((a, b) => a + b, 0) : null;
}

/** Dollars as text: `$1.23`, `<$0.01`, or `unknown` (never $0) for null. */
function usdText(usd) {
  if (usd === null || usd === undefined) return 'unknown';
  return usd > 0 && usd < 0.005 ? '<$0.01' : `$${usd.toFixed(2)}`;
}

/** An estimated cost from the server: `$1.23`, `unknown`, or `>= $1.23 (partial)`. For a total, `withModels` also names the unpriced models. */
function costText(cost, withModels = false) {
  const text = cost?.partial && cost.usd != null ? `>= ${usdText(cost.usd)} (partial)` : usdText(cost?.usd);
  const models = withModels ? cost?.unpriced_models ?? [] : [];
  return models.length ? `${text}, ${models.length} unpriced: ${models.join(', ')}` : text;
}

const compactTokens = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(n));
const tokenCount = (n) => (n === undefined || n === null ? 'n/a' : n.toLocaleString('en-US'));

/** Ticket progress and fresh tokens as chips: the context a Run's row shows. */
function ContextChips({ run }) {
  const progress = ticketProgress(run);
  const tokens = usageTotal(run);
  return html`${progress
      ? html`<span class="chip">Tickets${' '}${progress.done}/${progress.total}<span class="visually-hidden">${' done'}</span></span>`
      : null}${tokens !== null
      ? html`<span class="chip">Tokens${' '}${compactTokens(tokens)}<span class="visually-hidden">${' fresh'}</span>${run.usage.status === 'partial' ? ' (partial)' : ''}</span>`
      : null}`;
}

/** Where a row's tokens were spent: a ticket, the rework, or (in Build) no ticket. */
function rowBucket(row) {
  if (row.ticket) return row.ticket;
  if (row.rework) return 'Rework';
  return row.stage === 'build' ? 'No ticket' : '';
}

/** A short name for a ticket link: #202 for an issue, pull request or work item, else the last part of the path. */
function shortRef(url) {
  const number = url.pathname.match(/(?:issues|pull|pullrequest|workitems\/edit)\/(\d+)\/?$/);
  if (number) return `#${number[1]}`;
  return url.pathname.split('/').filter(Boolean).at(-1) ?? url.host;
}

/** A row's ticket in the token table: a short link with the whole URL as its tooltip, or the text as it is. */
function TicketRef({ value }) {
  const url = httpUrl(value);
  return url
    ? html`<a href=${url.href} title=${url.href} target="_blank" rel="noopener noreferrer"
        ><span class="visually-hidden">Ticket </span>${shortRef(url)}</a
      >`
    : value;
}

/** The stage, model and ticket breakdown of a Run's token usage, with what is missing from it. */
function UsageDetail({ usage }) {
  const stageName = (id) => (GROUPS.find(([key]) => key === id) ?? [id, id])[1];
  const rows = Array.isArray(usage.rows) ? usage.rows : [];
  const total = rows.map(freshTokens).filter((n) => n !== null).reduce((a, b) => a + b, 0);
  const status =
    usage.status === 'tracked' ? 'Tracked' : usage.status === 'partial' ? 'Partial' : `Not tracked${usage.reason ? `: ${usage.reason}` : ''}`;
  // Fresh tokens and cache reads per model; a count no row reported stays null (n/a).
  const perModel = new Map();
  for (const row of rows) {
    const sum = perModel.get(row.model) ?? { fresh: null, cache_read: null };
    const fresh = freshTokens(row);
    if (fresh !== null) sum.fresh = (sum.fresh ?? 0) + fresh;
    if (row.cache_read != null) sum.cache_read = (sum.cache_read ?? 0) + row.cache_read;
    perModel.set(row.model, sum);
  }
  const models = [...perModel.entries()].sort(([a], [b]) => a.localeCompare(b));
  const noData = Array.isArray(usage.no_data) ? usage.no_data : [];
  const notes = Array.isArray(usage.notes) ? usage.notes : [];
  return html`<p>${status}</p>
    ${rows.length
      ? html`<div class="table-scroll" role="region" aria-label="Token usage by stage, model and ticket" tabindex="0">
            <table class="usage-table">
              <caption class="visually-hidden">Token usage by stage, model and ticket</caption>
              <thead>
                <tr>
                  <th scope="col" class="text">Stage</th>
                  <th scope="col" class="text">Model</th>
                  <th scope="col" class="text">Ticket</th>
                  <th scope="col">Fresh</th>
                  <th scope="col">Input</th>
                  <th scope="col">Cache read</th>
                  <th scope="col">Cache write</th>
                  <th scope="col">Output</th>
                  <th scope="col">Reasoning</th>
                  <th scope="col">Subagents</th>
                  <th scope="col">Est. cost</th>
                </tr>
              </thead>
              <tbody>
                ${rows.map(
                  (row, index) => html`<tr key=${index}>
                    <td class="text">${stageName(row.stage)}</td>
                    <td class="text wrap">${row.model}</td>
                    <td class="text wrap"><${TicketRef} value=${rowBucket(row)} /></td>
                    <td>${tokenCount(freshTokens(row))}</td>
                    <td>${tokenCount(row.input)}</td>
                    <td>${tokenCount(row.cache_read)}</td>
                    <td>${tokenCount(row.cache_write)}</td>
                    <td>${tokenCount(row.output)}</td>
                    <td>${tokenCount(row.reasoning)}</td>
                    <td>${row.subagents ? tokenCount(freshTokens(row.subagents)) : '-'}</td>
                    <td>${usdText(row.cost_usd)}</td>
                  </tr>`,
                )}
              </tbody>
              <tfoot>
                <tr>
                  <th scope="row" colspan="3" class="text">Total fresh</th>
                  <td>${tokenCount(total)}</td>
                  <td colspan="6"></td>
                  <td class="wrap">${costText(usage.cost, true)}</td>
                </tr>
              </tfoot>
            </table>
          </div>
          ${models.length
            ? html`<div class="table-scroll" role="region" aria-label="Token usage and estimated cost per model" tabindex="0">
                <table class="usage-table model-table">
                  <caption class="visually-hidden">Token usage and estimated cost per model</caption>
                  <thead>
                    <tr>
                      <th scope="col" class="text">Model</th>
                      <th scope="col">Fresh</th>
                      <th scope="col">Cache read</th>
                      <th scope="col">Est. cost</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${models.map(
                      ([model, tokens]) => html`<tr key=${model}>
                        <td class="text wrap">${model}</td>
                        <td>${tokenCount(tokens.fresh)}</td>
                        <td>${tokenCount(tokens.cache_read)}</td>
                        <td>${costText(usage.cost?.by_model?.[model])}</td>
                      </tr>`,
                    )}
                  </tbody>
                </table>
              </div>`
            : null}
          <p class="usage-note">
            Fresh tokens are input excluding cache reads, plus cache writes and output. Cache reads are shown beside
            them, not inside. Reasoning is part of output. n/a means the log did not report it; it is not zero. Est. cost is an estimate
            from Jidoka's price table; a model without a price is unknown, never zero, and a total that includes
            one is marked partial.
          </p>`
      : null}
    ${noData.length ? html`<p>No data for: ${noData.map(stageName).join(', ')}</p>` : null}
    ${notes.length ? html`<ul class="summaries">${notes.map((note, index) => html`<li key=${index}>${note}</li>`)}</ul>` : null}`;
}

const PR_STATE = { open: 'Open', draft: 'Draft', merged: 'Merged', closed: 'Closed' };

/** The state of the Run's pull request in words, once the server has read it for that pull request. */
function PrStateLabel({ run }) {
  const status = run.pr_status;
  if (!status || status.url !== run.pr || !PR_STATE[status.state]) return null;
  return html`<span class=${`pr-state ${status.state}`}>${PR_STATE[status.state]}</span>`;
}

function ContextLinks({ run }) {
  const named = (label) => html`${label}<span class="visually-hidden">${` of ${run.title}`}</span>`;
  const pr = run.pr ? html`<${RefLink} value=${run.pr} label=${named('Pull request')} />` : null;
  const spec = !run.spec
    ? null
    : httpUrl(run.spec)
      ? html`<${RefLink} value=${run.spec} label=${named('Spec')} />`
      : isReadable(run.spec)
        ? html`<${ReaderLink} run=${run} reference=${run.spec} label=${`View spec of ${run.title}`}>View</${ReaderLink}>`
        : null;
  return pr || spec
    ? html`<span class="links">${pr}${spec}</span>`
    : null;
}

const reworkLabel = (count) =>
  count <= REWORK_CAP ? `${count}/${REWORK_CAP}` : `${count} (past the cap of ${REWORK_CAP})`;

/** "just now", "5m ago", "3h ago", "2d ago". */
function ago(ms) {
  const minutes = Math.floor(Math.max(0, ms) / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function Updated({ run, now }) {
  const ts = timestamp(run);
  if (ts === null) return html`<span class="updated">Updated: not reported</span>`;
  const date = new Date(ts);
  return html`<span class="updated">
    Updated${' '}<time datetime=${date.toISOString()} title=${localDateTime(date)}
      >${ago(now - ts)}</time
    >
  </span>`;
}

const STATIONS = ['define', 'build', 'check', 'finish'];

/**
 * One active Run on the line: its title (opens the detail), a rail of the four stations with a marker at the
 * current one, the position in words, and a meta line. A blocked Run is marked amber, with a word, not only a colour.
 */
function LineRow({ run, now, onOpen }) {
  const index = STATIONS.indexOf(run.stage);
  const blocked = needsYou(run);
  const station = index === -1 ? String(run.stage) : GROUPS[index][1];
  const where = blocked ? `${station}, blocked` : station;
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  return html`<li class=${blocked ? 'line-row blocked' : 'line-row'} style=${`--station: ${index}`}>
    <h3 class="line-title">
      <button type="button" class="line-open" data-run-key=${runKey(run)} onClick=${() => onOpen(runKey(run))}>
        ${run.title}
      </button>
    </h3>
    <div class="rail" aria-hidden="true">
      ${STATIONS.map(
        (id, at) => html`<span key=${id} class=${at < index ? 'station past' : at === index ? 'station current' : 'station'}></span>`,
      )}
      ${index === -1 ? null : html`<span class="rail-marker"></span>`}
    </div>
    <p class="where">${where}</p>
    <div class="meta">
      <span class="meta-project"><${ProjectName} path=${projectOf(run)} /></span>
      <${ContextChips} run=${run} />
      ${rework !== null ? html`<span class="chip">Rework${' '}${reworkLabel(rework)}</span>` : null}
      <${ContextLinks} run=${run} /><${PrStateLabel} run=${run} />
      <${Updated} run=${run} now=${now} />
    </div>
    ${run.activity ? html`<p class="activity">${run.activity}</p>` : null}
  </li>`;
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

/** A time as the sheet shows it, with the exact moment for machines. */
function SheetTime({ at, now }) {
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return null;
  return html`<time datetime=${date.toISOString()}>${sheetTime(at, now)}</time>`;
}

/** A small text button that copies `value`, then says so for a moment, and for screen readers in a live region. */
function CopyButton({ value, what }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);
  const copy = async () => {
    let done = false;
    try {
      await navigator.clipboard.writeText(value);
      done = true;
    } catch {
      /* no clipboard permission: fall back to selecting a throwaway field */
    }
    if (!done) {
      const field = document.createElement('textarea');
      field.value = value;
      field.setAttribute('readonly', '');
      field.style.position = 'fixed';
      field.style.opacity = '0';
      document.body.append(field);
      field.select();
      try {
        done = document.execCommand('copy');
      } catch {
        done = false;
      }
      field.remove();
    }
    if (done) setCopied(true);
  };
  return html`<span class="copy"
    ><button type="button" class="text-button" onClick=${copy}>
      ${copied ? 'Copied' : 'Copy'}<span class="visually-hidden">${` ${what}`}</span>
    </button>
    <span class="visually-hidden" role="status">${copied ? `Copied ${what}` : ''}</span></span
  >`;
}

/** The skills a stage used: quiet word chips, one per distinct skill. */
function SkillChips({ reports }) {
  const skills = [...new Set(reports.filter((r) => r.kind === 'skill').map((r) => r.text))];
  return skills.length
    ? html`<ul class="skills" aria-label="Skills used">${skills.map((skill) => html`<li key=${skill} class="skill">${skill}</li>`)}</ul>`
    : null;
}

/** Activity reports as a list of when and what. */
function ReportList({ reports, now, showTicket = false }) {
  const activity = reports.filter((r) => r.kind !== 'skill');
  return activity.length
    ? html`<ul class="reports">
        ${activity.map(
          (report, index) => html`<li key=${index}>
            <${SheetTime} at=${report.at} now=${now} /><span>${showTicket && report.ticket != null ? html`<${TicketRef} value=${String(report.ticket)} />${' '}` : null}${report.text}</span>
          </li>`,
        )}
      </ul>`
    : null;
}

/** Fresh tokens spent in one stage, or null when the usage has no row for it. */
function stageTokens(run, id) {
  const rows = Array.isArray(run.usage?.rows) ? run.usage.rows.filter((row) => row.stage === id) : [];
  const counts = rows.map(freshTokens).filter((n) => n !== null);
  return counts.length ? counts.reduce((a, b) => a + b, 0) : null;
}

const STATION_NAME = Object.fromEntries(GROUPS);
const STATION_STATE = { done: 'done', current: 'current', blocked: 'current, blocked', ahead: 'not reached' };

/** Where a station stands for a Run: done, current, blocked while it is the current one of a blocked Run, or ahead. */
function stationState(run, at) {
  if (run.status === 'done') return 'done';
  const current = STATIONS.indexOf(run.stage);
  if (at < current) return 'done';
  if (at === current) return run.status === 'blocked' ? 'blocked' : 'current';
  return 'ahead';
}

/** `value` when it is an array, else an empty one: stored runs may lack a list or hold something else. */
const listOf = (value) => (Array.isArray(value) ? value : []);

/** Where each report belongs: a run report by its stage, a ticket report by its listed ticket, anything else in `other`. */
function placeReports(run) {
  const runReports = listOf(run.run_reports);
  const ticketReports = listOf(run.ticket_reports);
  const tickets = listOf(run.tickets);
  const listed = (r) => tickets.some((t) => t.reference === r.ticket);
  return {
    byStage: (id) => runReports.filter((r) => r.stage === id),
    byTicket: (reference) => ticketReports.filter((r) => r.ticket === reference),
    other: [...runReports.filter((r) => !STATIONS.includes(r.stage)), ...ticketReports.filter((r) => !listed(r))],
  };
}

/** A duration in plain words: "under 1 min", "12 min", "1 h 5 min". */
export function durationWords(ms) {
  const minutes = Math.floor(ms / 60000);
  if (!(minutes >= 1)) return 'under 1 min';
  if (minutes < 60) return `${minutes} min`;
  const rest = minutes % 60;
  return rest ? `${Math.floor(minutes / 60)} h ${rest} min` : `${Math.floor(minutes / 60)} h`;
}

/**
 * The passes through the stages that the run's timeline records, oldest first: consecutive Build marks that differ
 * only by ticket form one pass that lists those tickets. `unstartedTickets` are tickets no Build pass started and
 * that have no first Build pass to wait in. `now` ends the last pass of a run that is not finished.
 */
export function timelinePasses(run, now) {
  const marks = listOf(run.timeline)
    .filter((mark) => mark && Number.isFinite(mark.at) && STATIONS.includes(mark.stage))
    .sort((a, b) => a.at - b.at);
  const allTickets = listOf(run.tickets);
  const passes = [];
  for (const mark of marks) {
    const rework = Number.isInteger(mark.rework) && mark.rework > 0 ? mark.rework : null;
    const activity = typeof mark.activity === 'string' && mark.activity ? mark.activity : null;
    const previous = passes[passes.length - 1];
    if (previous && mark.stage === 'build' && previous.stage === 'build' && previous.rework === rework) {
      previous.refs.push(mark.ticket);
      if (activity && !previous.activities.includes(activity)) previous.activities.push(activity);
      continue;
    }
    passes.push({ stage: mark.stage, rework, start: mark.at, activities: activity ? [activity] : [], refs: [mark.ticket] });
  }
  if (!passes.length) return { passes, unstartedTickets: [] };
  const finished = run.status === 'done' ? (Number.isFinite(run.finished_at) ? run.finished_at : timestamp(run)) : now;
  const used = new Set(passes.flatMap((pass) => pass.refs).filter((ref) => ref != null));
  // Tickets no Build pass has started yet are still pending work of the first Build pass, or stand apart without one.
  const notStarted = allTickets.filter((ticket) => !used.has(ticket.reference));
  const firstBuildPass = passes.find((pass) => pass.stage === 'build' && pass.rework === null);
  passes.forEach((pass, index) => {
    const next = passes[index + 1];
    const end = next ? next.start : finished;
    pass.duration = Number.isFinite(end) ? Math.max(0, end - pass.start) : null;
    pass.end = next ? next.start : Infinity;
    pass.tickets = allTickets.filter((ticket) => pass.refs.includes(ticket.reference) || (pass === firstBuildPass && notStarted.includes(ticket)));
    pass.sentBack = pass.stage === 'check' && next?.stage === 'build' && next.rework !== null;
    pass.lastOfStage = !passes.slice(index + 1).some((later) => later.stage === pass.stage);
    pass.state = next ? 'done' : run.status === 'done' ? 'done' : run.status === 'blocked' ? 'blocked' : 'current';
  });
  return { passes, unstartedTickets: firstBuildPass ? [] : notStarted };
}

const PAUSE_WORDS = { usage_limit: 'Usage limit reached', compaction: 'Context compaction' };

/**
 * The run's moments that carry a time and are shown as lines on the timeline: blocked and resumed events, pauses the
 * agent's log shows (a usage limit or a compaction), and idle gaps. Pauses and idle gaps become `pause` and `idle` items.
 */
function validEvents(run) {
  const events = listOf(run.events).filter((event) => event && Number.isFinite(event.at));
  const usage = run.usage ?? {};
  const pauses = listOf(usage.pauses)
    .filter((pause) => pause && Number.isFinite(pause.at))
    .map((pause) => ({ kind: 'pause', at: pause.at, resumedAt: Number.isFinite(pause.resumed_at) ? pause.resumed_at : null, name: PAUSE_WORDS[pause.kind] ?? 'Paused' }));
  const idle = listOf(usage.idle)
    .filter((gap) => gap && Number.isFinite(gap.from) && Number.isFinite(gap.to) && gap.to > gap.from)
    .map((gap) => ({ kind: 'idle', at: gap.from, to: gap.to }));
  return [...events, ...pauses, ...idle].sort((a, b) => a.at - b.at);
}

/** A blocked or resumed moment, a pause or an idle gap as one line with its time. */
function EventLine({ event, now }) {
  if (event.kind === 'pause' || event.kind === 'idle') {
    const text = event.kind === 'idle'
      ? `Idle ${durationWords(event.to - event.at)}`
      : `${event.name} · ${event.resumedAt !== null ? `resumed after ${durationWords(event.resumedAt - event.at)}` : 'not resumed yet'}`;
    return html`<p class=${`pass-event ${event.kind}`}>${text}${' · '}<${SheetTime} at=${event.at} now=${now} /></p>`;
  }
  return html`<p class=${`pass-event ${event.kind === 'blocked' ? 'blocked' : 'resumed'}`}>
    ${event.kind === 'blocked' ? (event.reason ? `Blocked: ${event.reason}` : 'Blocked') : 'Resumed'}${' · '}<${SheetTime} at=${event.at} now=${now} />
  </p>`;
}

/** Appends `item` to the list stored under `key`. */
function push(map, key, item) {
  map.set(key, [...(map.get(key) ?? []), item]);
}

/**
 * Where each report and event belongs on a timeline. A report with a stage goes in the last pass of that stage that
 * started at or before its `at`, else the nearest pass of that stage; one without a stage (older data) goes in the
 * pass whose time window holds its `at`. The rest are in `other`. An event goes in the pass whose window holds it, else the nearest pass of its stage, else in `otherEvents`.
 */
function placeByTime(run, passes, unstartedTickets) {
  const tickets = listOf(run.tickets);
  const placed = passes.map(() => ({ reports: [], byTicket: new Map(), events: [] }));
  const unstarted = new Map();
  const other = [];
  const windowIndex = (item) => passes.findIndex((pass) => Number.isFinite(item.at) && item.at >= pass.start && item.at < pass.end);
  // The last pass of the item's stage that started at or before it, else the first pass of that stage.
  const stageIndex = (item) => {
    let first = -1;
    let lastBefore = -1;
    passes.forEach((pass, index) => {
      if (pass.stage !== item.stage) return;
      if (first === -1) first = index;
      if (!(pass.start > item.at)) lastBefore = index;
    });
    return lastBefore !== -1 ? lastBefore : first;
  };
  const hasStage = (report) => STATIONS.includes(report.stage);
  const passIndex = (report) => (hasStage(report) ? stageIndex(report) : windowIndex(report));
  for (const report of listOf(run.run_reports)) {
    const at = passIndex(report);
    (at === -1 ? other : placed[at].reports).push(report);
  }
  for (const report of listOf(run.ticket_reports)) {
    const at = passIndex(report);
    const listed = tickets.find((ticket) => ticket.reference === report.ticket);
    const waiting = listed && unstartedTickets.includes(listed) && (at === -1 || !hasStage(report));
    if (!listed) other.push(report);
    else if (waiting) push(unstarted, listed.reference, report);
    else if (at === -1) other.push(report);
    else if (passes[at].tickets.includes(listed)) push(placed[at].byTicket, listed.reference, report);
    else placed[at].reports.push(report);
  }
  const otherEvents = [];
  for (const event of validEvents(run)) {
    const inWindow = windowIndex(event);
    const at = inWindow !== -1 ? inWindow : stageIndex(event);
    (at === -1 ? otherEvents : placed[at].events).push(event);
  }
  return { placed, other, otherEvents, unstarted };
}

function TicketList({ run, tickets, byTicket, now }) {
  return html`<ol class="ticket-list">
    ${tickets.map((ticket) => {
      const own = byTicket(ticket.reference);
      const current = ticket.status !== 'done' && run.current_ticket != null && run.current_ticket === ticket.reference;
      return html`<li key=${ticket.reference} class=${ticket.status === 'done' ? 'ticket done' : 'ticket'}>
        <p class="ticket-line">
          <span class="ticket-status">${current ? TICKET_STATUS['in-progress'] : (TICKET_STATUS[ticket.status] ?? String(ticket.status))}</span>
          <span class="ticket-name"><${TicketRef} value=${String(ticket.reference)} />${' '}${isReadable(ticket.reference)
            ? html`<${ReaderLink} run=${run} reference=${String(ticket.reference)}>${ticket.title}</${ReaderLink}>`
            : ticket.title}</span>
        </p>
        <${SkillChips} reports=${own} />
        <${ReportList} reports=${own} now=${now} />
      </li>`;
    })}
  </ol>`;
}

/** One pass through a stage on the route: when it started and how long it ran, what it started with, then what was recorded in it. */
function Pass({ run, pass, placed, now }) {
  const label = pass.rework === null ? STATION_NAME[pass.stage] : `${STATION_NAME[pass.stage]} · Rework ${pass.rework <= REWORK_CAP ? pass.rework : `${pass.rework} (past the cap of ${REWORK_CAP})`}`;
  const summary = pass.lastOfStage ? run.summaries?.[pass.stage] : null;
  const progress = pass.tickets.length ? { done: pass.tickets.filter((t) => t.status === 'done').length, total: pass.tickets.length } : null;
  const classes = ['stop', pass.state, pass.rework !== null ? 'rework-pass' : '', pass.sentBack ? 'sent-back' : ''].filter(Boolean).join(' ');
  return html`<li class=${classes}>
    <span class="mark" aria-hidden="true"></span>
    <div class="stop-head">
      <h3>${label}<span class="visually-hidden">${`, ${STATION_STATE[pass.state]}`}</span></h3>
      ${progress ? html`<span class="stop-meta"><span class="ticket-count">${progress.done} of ${progress.total} tickets done</span></span>` : null}
    </div>
    <p class="pass-when">
      <${SheetTime} at=${pass.start} now=${now} />${pass.duration !== null ? html`${' · '}<span class="pass-duration">${durationWords(pass.duration)}</span>` : null}
    </p>
    ${pass.activities.map((activity) => html`<p key=${activity} class="pass-activity">${activity}</p>`)}
    ${placed.events.map((event, index) => html`<${EventLine} key=${index} event=${event} now=${now} />`)}
    ${summary ? html`<p class="stop-summary">${summary}</p>` : null}
    <${SkillChips} reports=${placed.reports} />
    <${ReportList} reports=${placed.reports} now=${now} showTicket=${true} />
    ${pass.tickets.length ? html`<${TicketList} run=${run} tickets=${pass.tickets} byTicket=${(ref) => placed.byTicket.get(ref) ?? []} now=${now} />` : null}
    ${pass.sentBack ? html`<p class="sent-back-note">sent back</p>` : null}
  </li>`;
}

/** One station of the route: its mark, name and tokens, then what the Run recorded there; a bare name when nothing. */
function Station({ run, id, name, at, now }) {
  const state = stationState(run, at);
  const placed = placeReports(run);
  const reports = placed.byStage(id);
  const tickets = id === 'build' ? listOf(run.tickets) : [];
  const progress = id === 'build' ? ticketProgress(run) : null;
  const rework = id === 'check' && Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  const summary = run.summaries?.[id];
  const tokens = stageTokens(run, id);
  const stageCost = run.usage?.cost?.by_stage?.[id];
  return html`<li class=${`stop ${state}`}>
    <span class="mark" aria-hidden="true"></span>
    <div class="stop-head">
      <h3>${name}<span class="visually-hidden">${`, ${STATION_STATE[state]}`}</span></h3>
      ${progress || tokens !== null || stageCost
        ? html`<span class="stop-meta">
            ${progress ? html`<span class="ticket-count">${progress.done} of ${progress.total} tickets done</span>` : null}
            ${tokens !== null ? html`<span class="stop-tokens">${compactTokens(tokens)}<span class="visually-hidden">${' fresh tokens'}</span></span>` : null}
            ${stageCost ? html`<span class="stop-cost"><span class="visually-hidden">Estimated cost </span>${costText(stageCost, true)}</span>` : null}
          </span>`
        : null}
    </div>
    ${summary ? html`<p class="stop-summary">${summary}</p>` : null}
    ${rework !== null ? html`<p class="rework"><span class="chip">Rework${' '}${reworkLabel(rework)}</span></p>` : null}
    <${SkillChips} reports=${reports} />
    <${ReportList} reports=${reports} now=${now} />
    ${tickets.length ? html`<${TicketList} run=${run} tickets=${tickets} byTicket=${placed.byTicket} now=${now} />` : null}
  </li>`;
}

/** Reports that belong to no station, no pass or no listed ticket, and summaries of stages with no pass, so nothing is dropped silently. */
function OtherReports({ reports, events, summaries, now }) {
  if (!reports.length && !events.length && !summaries.length) return null;
  return html`<section class="other-reports" aria-labelledby="other-reports-title">
    <h3 id="other-reports-title">Other reports</h3>
    ${summaries.map(([name, text]) => html`<p key=${name} class="other-summary">${name}: ${text}</p>`)}
    ${events.map((event, index) => html`<${EventLine} key=${index} event=${event} now=${now} />`)}
    <${SkillChips} reports=${reports} />
    <${ReportList} reports=${reports} now=${now} showTicket=${true} />
  </section>`;
}

/** Tickets no pass started, when the timeline has no first Build pass to list them under. */
function UnstartedTickets({ run, tickets, byTicket, now }) {
  if (!tickets.length) return null;
  return html`<section class="unstarted-tickets" aria-labelledby="unstarted-title">
    <h3 id="unstarted-title">Tickets</h3>
    <${TicketList} run=${run} tickets=${tickets} byTicket=${(ref) => byTicket.get(ref) ?? []} now=${now} />
  </section>`;
}

/** The pull request as a link with its state in words, once the server has read that pull request. */
function PrLink({ run }) {
  if (!run.pr) return null;
  const url = httpUrl(run.pr);
  const status = run.pr_status;
  const known = status && status.url === run.pr;
  const state = known ? (PR_STATE[status.state] ? PR_STATE[status.state].toLowerCase() : 'state unknown') : null;
  return html`<span class="status-item">
    ${url ? html`<a href=${url.href} target="_blank" rel="noopener noreferrer">PR ${shortRef(url)}</a>` : `PR ${run.pr}`}
    ${state ? html`${' '}<span class=${`pr-word ${known && status.state}`} title=${status.state === 'unknown' ? status.reason : null}>${state}</span>` : null}
    ${known && status.state === 'unknown' && status.reason ? html`:${' '}<span class="pr-reason">${status.reason}</span>` : null}
  </span>`;
}

/** Asks the server to read the Run's pull request from its host now; the Run follows a merge or a close. */
async function syncRun(id) {
  const response = await fetch(`/api/runs/${encodeURIComponent(id)}/sync`, {
    method: 'POST',
    headers: { 'X-Session-Token': sessionToken() },
  });
  if (!response.ok) throw new Error('Sync failed');
  return response.json();
}

const SYNC_RESULT = {
  open: 'Pull request is still open.',
  draft: 'Pull request is still a draft.',
  merged: 'Pull request is merged; the run is finished.',
  closed: 'Pull request was closed without merging; the run is blocked.',
};

/** Reads the pull request now instead of waiting for the next automatic check, and says what it found. */
function CheckPrButton({ run, onSynced }) {
  const [check, setCheck] = useState({ busy: false, text: '' });
  // A finished run has nothing to follow, unless this check just finished it and the result is still shown.
  if (!run.pr || (run.status === 'done' && !check.text)) return null;
  const start = async () => {
    setCheck({ busy: true, text: 'Checking the pull request…' });
    try {
      const status = await syncRun(run.id);
      const text =
        SYNC_RESULT[status.state] ?? `The pull request could not be read${status.reason ? `: ${status.reason}` : '.'}`;
      setCheck({ busy: false, text });
      await onSynced();
    } catch {
      setCheck({ busy: false, text: 'The dashboard could not check the pull request.' });
    }
  };
  return html`<span class="status-item">
    <button type="button" class="text-button" disabled=${check.busy} onClick=${start}>
      Check PR<span class="visually-hidden">${` of ${run.title}`}</span>
    </button>
    <span class="pr-reason" role="status">${check.text}</span>
  </span>`;
}

/** The Run's route card in a native modal dialog on the right. The browser handles focus containment and Escape. */
function Detail({ run, now, onClose, onDelete, onSynced }) {
  const dialog = useRef(null);
  // The listener below is added once; it calls whatever `onClose` is current.
  const closing = useRef(onClose);
  closing.current = onClose;
  // Whether the current press began on the backdrop, so a drag out of the sheet is not a backdrop click.
  const pressedBackdrop = useRef(false);
  useEffect(() => {
    const element = dialog.current;
    if (!element.open) element.showModal();
    // Escape has no hook of its own: the browser closes the dialog and fires `close`.
    const onNativeClose = () => closing.current();
    element.addEventListener('close', onNativeClose);
    return () => element.removeEventListener('close', onNativeClose);
  }, []);
  // Close the dialog and tell the page now, rather than waiting for the `close` event.
  const dismiss = () => {
    dialog.current.close();
    closing.current();
  };
  const ts = timestamp(run);
  const active = run.status !== 'done';
  const blocked = run.status === 'blocked';
  const spec = run.spec ? httpUrl(run.spec) : null;
  const localSpec = run.spec && !spec ? String(run.spec) : null;
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  const total = usageTotal(run);
  const listed = Array.isArray(run.tickets) && run.tickets.some((t) => t.reference === run.current_ticket);
  const { passes, unstartedTickets } = timelinePasses(run, now);
  const placed = passes.length ? placeByTime(run, passes, unstartedTickets) : null;
  const otherReports = placed ? placed.other : placeReports(run).other;
  const otherEvents = placed ? placed.otherEvents : validEvents(run);
  // A summary is kept per stage; one whose stage has no pass in the timeline (it may have been cut short) shows here.
  const orphanSummaries = placed
    ? GROUPS.filter(([id]) => run.summaries?.[id] && !passes.some((pass) => pass.stage === id)).map(([id, name]) => [name, run.summaries[id]])
    : [];
  return html`<dialog
    ref=${dialog}
    class="detail"
    aria-labelledby="detail-title"
    onPointerDown=${(event) => {
      pressedBackdrop.current = event.target === event.currentTarget;
    }}
    onClick=${(event) => {
      // A click on the backdrop lands on the dialog itself; the body fills the rest of the sheet.
      if (event.target === event.currentTarget && pressedBackdrop.current) dismiss();
      pressedBackdrop.current = false;
    }}
  >
    <div class="detail-body">
      <div class="detail-bar">
        <button type="button" class="toggle detail-close" onClick=${dismiss}>
          <span class="detail-close-wide">Close</span><span class="detail-close-compact">Back</span>
        </button>
      </div>
      <h2 id="detail-title">${run.title}</h2>
      <p class="detail-sub">
        <span><${ProjectName} path=${projectOf(run)} /></span>
        ${run.mode ? html`<span>${run.mode}</span>` : null}
        ${run.branch ? html`<span class="branch"><code>${run.branch}</code><${CopyButton} value=${run.branch} what="branch" /></span>` : null}
      </p>
      ${active && run.activity ? html`<p class="detail-activity">${run.activity}</p>` : null}
      ${active && run.current_ticket && !listed ? html`<p class="detail-ticket">Current ticket: ${run.current_ticket}</p>` : null}
      <div class=${blocked ? 'status-row blocked' : 'status-row'}>
        <span class=${STATUSES.includes(run.status) ? `badge ${run.status}` : 'badge'}>${statusLabel(run.status)}</span>
        <${PrLink} run=${run} />
        <${CheckPrButton} run=${run} onSynced=${onSynced} />
        ${spec ? html`<span class="status-item"><a href=${spec.href} target="_blank" rel="noopener noreferrer">Spec ${shortRef(spec)}</a></span>` : null}
        ${localSpec ? html`<span class="status-item">Spec ${isReadable(localSpec) ? html`<${ReaderLink} run=${run} reference=${localSpec}>View</${ReaderLink}>` : localSpec}</span>` : null}
        ${rework !== null ? html`<span class="status-item">Rework${' '}${reworkLabel(rework)}</span>` : null}
        ${ts !== null ? html`<span class="status-item status-updated">Updated${' '}<${SheetTime} at=${ts} now=${now} /></span>` : null}
        ${blocked && run.blocking_reason ? html`<p class="status-reason">${run.blocking_reason}</p>` : null}
      </div>
      <ol class="route" aria-label="Route">
        ${placed
          ? passes.map((pass, index) => html`<${Pass} key=${index} run=${run} pass=${pass} placed=${placed.placed[index]} now=${now} />`)
          : GROUPS.map(([id, name], at) => html`<${Station} key=${id} run=${run} id=${id} name=${name} at=${at} now=${now} />`)}
      </ol>
      ${placed ? html`<${UnstartedTickets} run=${run} tickets=${unstartedTickets} byTicket=${placed.unstarted} now=${now} />` : null}
      <${OtherReports} reports=${otherReports} events=${otherEvents} summaries=${orphanSummaries} now=${now} />
      ${run.usage
        ? html`<details class="usage" open=${active}>
            <summary>
              Token usage${total !== null ? html`<span class="usage-total">${tokenCount(total)}${' fresh'}</span>` : null}${run.usage.cost
                ? html`<span class="usage-total">${'Est. cost '}${costText(run.usage.cost, true)}</span>`
                : null}
            </summary>
            <${UsageDetail} usage=${run.usage} />
          </details>`
        : null}
      <div class="detail-foot">
        <dl class="ids">
          ${run.repository
            ? html`<div><dt>Repository</dt><dd><code>${run.repository}</code><${CopyButton} value=${run.repository} what="repository" /></dd></div>`
            : null}
          <div><dt>Run id</dt><dd><code>${run.id}</code><${CopyButton} value=${String(run.id)} what="run id" /></dd></div>
        </dl>
        <div class="foot-line">
          <p class="note">Skills and activity are reported by the agent and may be incomplete.</p>
          <button type="button" class="text-button detail-delete" onClick=${() => onDelete(run)}>
            Delete run…<span class="visually-hidden">${` ${run.title}`}</span>
          </button>
        </div>
      </div>
    </div>
  </dialog>`;
}

/** Exact timestamps use day-first order while retaining the browser's local timezone. */
function localDateTime(date) {
  return date.toLocaleString('en-GB');
}

/** The day a Run shipped, by its last update: Today, Yesterday, the date, or Undated without a time. */
function dayLabel(ts, now) {
  if (ts === null) return 'Undated';
  const day = (ms) => new Date(ms).setHours(0, 0, 0, 0);
  const days = Math.round((day(now) - day(ts)) / 86400000);
  if (days <= 0) return 'Today';
  if (days === 1) return 'Yesterday';
  return new Date(ts).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
}

/** Finished Runs in a compact table, newest first, grouped by day. Clear shipped removes their records after confirming. */
function ShippedLog({ entries, now, onOpen, onClear }) {
  const groups = [];
  for (const entry of entries) {
    const label = dayLabel(timestamp(entry.run), now);
    if (groups.at(-1)?.label !== label) groups.push({ label, runs: [] });
    groups.at(-1).runs.push(entry.run);
  }
  return html`<section class="shipped" aria-labelledby="shipped-title">
    <div class="shipped-head">
      <h2 id="shipped-title">Shipped<span class="count">${` (${entries.length})`}</span></h2>
      <button type="button" class="text-button" onClick=${onClear}>
        Clear shipped…<span class="visually-hidden">${` ${entries.length} runs`}</span>
      </button>
    </div>
    <div class="table-scroll" role="region" aria-labelledby="shipped-title" tabindex="0">
      <table class="shipped-table">
        <caption class="visually-hidden">Shipped runs, newest first, grouped by day</caption>
        <thead>
          <tr>
            <th scope="col">Run</th>
            <th scope="col">Project</th>
            <th scope="col">Pull request</th>
            <th scope="col">Fresh tokens</th>
            <th scope="col">Shipped</th>
          </tr>
        </thead>
        ${groups.map(
          (group) => html`<tbody key=${group.label}>
            <tr class="day">
              <th scope="rowgroup" colspan="5">${group.label}</th>
            </tr>
            ${group.runs.map((run) => {
              const tokens = usageTotal(run);
              const ts = timestamp(run);
              return html`<tr key=${runKey(run)}>
                <td>
                  <button type="button" class="link log-open" data-run-key=${runKey(run)} onClick=${() => onOpen(runKey(run))}>
                    ${run.title}
                  </button>
                </td>
                <td><${ProjectName} path=${projectOf(run)} /></td>
                <td>${run.pr ? html`<${RefLink} value=${run.pr} label=${html`Pull request<span class="visually-hidden">${` of ${run.title}`}</span>`} />` : html`<span class="not-set">None</span>`}${' '}<${PrStateLabel} run=${run} /></td>
                <td class="num">${tokens === null ? 'n/a' : compactTokens(tokens)}</td>
                <td>
                  ${ts === null
                    ? html`<span class="not-set">Not reported</span>`
                    : html`<time datetime=${new Date(ts).toISOString()} title=${localDateTime(new Date(ts))}>${ago(now - ts)}</time>`}
                </td>
              </tr>`;
            })}
          </tbody>`,
        )}
      </table>
    </div>
  </section>`;
}

/** Shown instead of the board while there are no Runs. */
function Welcome({ onHelp }) {
  return html`<section class="welcome" aria-labelledby="welcome-title">
    <h2 id="welcome-title">Welcome to Jidoka</h2>
    <p>
      No Runs yet. In your project, ask your coding agent to run <code>/jidoka</code> to select a workflow, or choose
      <code>/jidoka-full</code>, <code>/jidoka-balanced</code>, or <code>/jidoka-fast</code> directly. Its Run appears here and follows the work from Define to a pull
      request.
    </p>
    <p>Not sure what the Stages, Needs you or Rework mean? <button type="button" class="link" onClick=${onHelp}>Read the Help</button>.</p>
  </section>`;
}

/** A native modal asking before something is deleted. Cancel has focus first; closing returns focus to what opened it. */
function Confirm({ title, children, confirmLabel, onConfirm, onCancel }) {
  const dialog = useRef(null);
  const cancelling = useRef(onCancel);
  cancelling.current = onCancel;
  useEffect(() => {
    const element = dialog.current;
    if (!element.open) element.showModal();
    // Escape closes the dialog and fires `close`.
    const onNativeClose = () => cancelling.current();
    element.addEventListener('close', onNativeClose);
    return () => element.removeEventListener('close', onNativeClose);
  }, []);
  return html`<dialog ref=${dialog} class="confirm" aria-labelledby="confirm-title">
    <h2 id="confirm-title">${title}</h2>
    ${children}
    <div class="confirm-buttons">
      <button type="button" class="toggle" autofocus onClick=${() => dialog.current.close()}>Cancel</button>
      <button type="button" class="toggle danger" onClick=${onConfirm}>${confirmLabel}</button>
    </div>
  </dialog>`;
}

/** How often the page asks whether the same server is still running. */
const SERVER_CHECK_MS = 5000;

/** The process id of the server that served this page. */
const servedBy = () => document.querySelector('meta[name="server-pid"]')?.content ?? '';

/** Whether something the visitor is reading or confirming is open: a dialog or the Projects menu. */
const somethingOpen = () => Boolean(document.querySelector('dialog[open], details.projects-menu[open]'));

/**
 * Reloads the page once another server answers, such as after an update restarted it, so the visitor always sees
 * the latest dashboard and holds a valid session token. It waits while a dialog or the Projects menu is open, and
 * while the server cannot be reached.
 */
function useReloadOnNewServer() {
  useEffect(() => {
    const page = servedBy();
    if (!page) return;
    let changed = false;
    const timer = setInterval(async () => {
      if (!changed) {
        try {
          const response = await fetch('/api/info', { cache: 'no-store' });
          if (!response.ok) return;
          const info = await response.json();
          changed = String(info.pid) !== page;
        } catch {
          return; /* the server is down, for instance while it restarts: ask again later */
        }
      }
      if (changed && !somethingOpen()) location.reload();
    }, SERVER_CHECK_MS);
    return () => clearInterval(timer);
  }, []);
}

/** The token this server put in the page; deleting needs it. */
const sessionToken = () => document.querySelector('meta[name="session-token"]')?.content ?? '';

/** Deletes the Run record; a Run that is already gone counts as deleted. */
async function deleteRun(id) {
  const response = await fetch(`/api/runs/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    headers: { 'X-Session-Token': sessionToken() },
  });
  return response.ok || response.status === 404;
}

/** Polls the Run list; `runs` stays null until the first answer and `failed` is set while the server is unreachable. */
function useRuns() {
  const [state, setState] = useState({ runs: null, failed: false, lastOk: null });
  const reload = useRef(() => {});
  useEffect(() => {
    let active = true;
    async function refresh() {
      try {
        const response = await fetch('/api/runs', { cache: 'no-store' });
        if (!response.ok) throw new Error('Failed to load runs');
        const data = await response.json();
        if (!Array.isArray(data)) throw new Error('Unexpected response');
        if (active) setState({ runs: data, failed: false, lastOk: Date.now() });
      } catch {
        if (active) setState((current) => ({ ...current, failed: true }));
      }
    }
    reload.current = refresh;
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);
  return { ...state, reload: () => reload.current() };
}

/** Whether the page can reach the server: a dot that pulses when the state changes, with the state in words. */
function Connection({ failed, lastOk, now }) {
  const state = failed ? 'offline' : lastOk === null ? 'connecting' : 'connected';
  const label = { offline: 'Offline', connecting: 'Connecting…', connected: 'Connected' }[state];
  const age =
    lastOk === null
      ? ''
      : state === 'offline'
        ? `last contact ${ago(now - lastOk)}`
        : `updated ${ago(now - lastOk)}`;
  return html`<p class=${`connection ${state}`}>
    <span class="pulse" key=${state} aria-hidden="true"></span>
    <span role="status">${label}</span>${age ? html`<span class="connection-age">${` · ${age}`}</span>` : null}
  </p>`;
}

/** The current time, refreshed so relative times keep counting. */
function useNow() {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), NOW_MS);
    return () => clearInterval(timer);
  }, []);
  return now;
}

const TITLE = 'Jidoka';
/** A Run needs the user when it is blocked. */
const needsYou = (run) => run.status === 'blocked';


const supportsNotifications = () => typeof Notification === 'function';
const permission = () => (supportsNotifications() ? Notification.permission : 'unsupported');

function storedNotifications() {
  try {
    return localStorage.getItem('notifications') === 'on';
  } catch {
    return false;
  }
}

/**
 * Opt-in browser notifications "Needs you in <Project>: <title>". The Runs already waiting on the first
 * load are only remembered; a notification goes out only for a Run that starts waiting after that.
 * `state` is 'unsupported', 'denied', 'off' or 'on'.
 */
function useNotifications(runs, names, hiddenProjects) {
  const [wanted, setWanted] = useState(storedNotifications);
  const [perm, setPerm] = useState(permission);
  const state = perm === 'unsupported' || perm === 'denied' ? perm : wanted && perm === 'granted' ? 'on' : 'off';
  const known = useRef(null);
  useEffect(() => {
    if (!runs) return;
    const waitingNow = runs.filter(needsYou);
    if (known.current !== null && state === 'on') {
      for (const run of waitingNow) {
        if (known.current.has(runKey(run))) continue;
        if (hiddenProjects.has(projectOf(run))) continue;
        try {
          const note = new Notification(`Needs you in ${names.get(projectOf(run)) ?? projectOf(run)}: ${run.title}`, { tag: `needs-you-${runKey(run)}` });
          note.onclick = () => window.focus();
        } catch {
          /* the browser refused to show it: nothing else to do */
        }
      }
    }
    known.current = new Set(waitingNow.map(runKey));
  }, [runs]);
  const remember = (on) => {
    setWanted(on);
    try {
      if (on) localStorage.setItem('notifications', 'on');
      else localStorage.removeItem('notifications');
    } catch {
      /* the choice still applies for this visit */
    }
  };
  const toggle = async () => {
    if (state === 'on') return remember(false);
    if (state !== 'off') return;
    let result = perm;
    if (result === 'default') {
      try {
        result = await Notification.requestPermission();
      } catch {
        result = 'denied';
      }
      setPerm(result);
    }
    remember(result === 'granted');
  };
  return { state, toggle };
}

function NotificationsToggle({ state, onToggle }) {
  const disabled = state === 'unsupported' || state === 'denied';
  const label =
    state === 'unsupported'
      ? 'Notifications are not supported by this browser'
      : state === 'denied'
        ? 'Notifications are blocked in the browser settings'
        : null;
  return html`<button
    type="button"
    class="toggle"
    aria-pressed=${String(state === 'on')}
    disabled=${disabled}
    title=${label}
    onClick=${onToggle}
  >
    Notify me${disabled ? html`<span class="visually-hidden">${` (${label})`}</span>` : null}
  </button>`;
}

/**
 * The andon strip: the top of the board says whether anything needs you. Quiet, it is one line with how many
 * Runs are on the line. Lit (amber), it lists each waiting Run with its Project and status reason; selecting one
 * opens its detail. Only the summary is a live region, so a change is announced once, not the whole list.
 */
function Andon({ waiting, active, filtered, onFilter, onOpen }) {
  const needs = waiting.length
    ? waiting.length === 1
      ? '1 run needs you'
      : `${waiting.length} runs need you`
    : 'Nothing needs you.';
  return html`<section class=${waiting.length ? 'andon lit' : 'andon'} aria-label="Needs you">
    <div class="andon-head">
      <p class="andon-summary">
        <span role="status">${needs}</span>${waiting.length ? null : ` ${active} ${active === 1 ? 'run' : 'runs'} on the line.`}
      </p>
      ${waiting.length || filtered
        ? html`<button type="button" class="toggle andon-filter" aria-pressed=${String(filtered)} onClick=${onFilter}>
            Show only these
          </button>`
        : null}
    </div>
    ${waiting.length
      ? html`<ul class="andon-list">
          ${waiting.map(
            (run) => html`<li key=${runKey(run)}>
              <button type="button" class="andon-run" data-run-key=${runKey(run)} onClick=${() => onOpen(runKey(run))}>
                <span class="andon-run-title">${run.title}</span>
                <span class="andon-run-project"><${ProjectName} path=${projectOf(run)} /></span>
                ${run.blocking_reason ? html`<span class="andon-run-reason">${run.blocking_reason}</span>` : null}
              </button>
            </li>`,
          )}
        </ul>`
      : null}
  </section>`;
}

/**
 * Every Project with Runs or hidden in this browser, each with Hide or Restore and Delete. Hiding is remembered in
 * this browser and keeps the Runs; Delete removes the Project's Run records after the confirm dialog.
 */
function ProjectsMenu({ projects, hiddenProjects, counts, onHide, onDelete }) {
  const names = useContext(ProjectNames);
  const menu = useRef(null);
  useEffect(() => {
    // A click outside closes the menu, as Escape does.
    const onOutside = (event) => {
      if (menu.current?.open && !menu.current.contains(event.target)) menu.current.open = false;
    };
    document.addEventListener('pointerdown', onOutside);
    return () => document.removeEventListener('pointerdown', onOutside);
  }, []);
  const onKeyDown = (event) => {
    if (event.key === 'Escape' && menu.current?.open) {
      menu.current.open = false;
      menu.current.querySelector('summary')?.focus();
    }
  };
  return html`<details class="projects-menu" ref=${menu} onKeyDown=${onKeyDown}>
    <summary>Projects (${projects.length})</summary>
    <div class="projects-panel">
    <ul class="projects-list">
      ${projects.map((project) => {
        const hidden = hiddenProjects.has(project);
        const name = names.get(project) ?? project;
        return html`<li key=${project} class=${hidden ? 'hidden-project' : null}>
          <span class="projects-name"><${ProjectName} path=${project} /></span>
          <span class="projects-count">${hidden ? 'Hidden' : `${counts.get(project) ?? 0} ${counts.get(project) === 1 ? 'run' : 'runs'}`}</span>
          <button
            type="button"
            class="text-button"
            aria-label=${`${hidden ? 'Restore' : 'Hide'} ${name}`}
            onClick=${() => onHide(project, !hidden)}
          >
            ${hidden ? 'Restore' : 'Hide'}
          </button>
          ${counts.get(project)
            ? html`<button
                type="button"
                class="text-button"
                aria-label=${`Delete ${name}`}
                onClick=${() => onDelete(project)}
              >
                Delete
              </button>`
            : null}
        </li>`;
      })}
    </ul>
    <p class="projects-note">Hidden projects are remembered in this browser. Their runs are kept.</p>
    </div>
  </details>`;
}

function App() {
  const { runs, failed, lastOk, reload } = useRuns();
  useReloadOnNewServer();
  const now = useNow();
  const [onlyWaiting, setOnlyWaiting] = useState(false);
  const [wanted] = useState(requestedProject);
  const [hiddenProjects, setHiddenProjects] = useState(storedHiddenProjects);
  const changeHidden = (project, hidden) => {
    const next = new Set(hiddenProjects);
    if (hidden) next.add(project);
    else next.delete(project);
    setHiddenProjects(next);
    try {
      localStorage.setItem('hidden-projects', JSON.stringify([...next]));
    } catch {
      /* the choice still applies until the page is reloaded */
    }
  };
  const visibleRuns = runs?.filter((run) => !hiddenProjects.has(projectOf(run))) ?? null;
  const waiting = visibleRuns ? visibleRuns.filter(needsYou).length : 0;
  const names = projectNames(runs ?? []);
  const notifications = useNotifications(runs, names, hiddenProjects);
  useEffect(() => {
    document.title = waiting ? `(${waiting}) ${TITLE}` : TITLE;
  }, [waiting]);
  // What the visitor is asked to confirm deleting.
  const [pending, setPending] = useState(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const closeHelp = () => {
    setHelpOpen(false);
    // The opener may be gone (the welcome message left); fall back to the header button.
    setTimeout(() => {
      if (!document.activeElement || document.activeElement === document.body)
        document.querySelector('.help-open')?.focus();
    });
  };
  const [deleteError, setDeleteError] = useState('');
  const confirmDelete = async () => {
    const targets = pending.runs;
    const project = pending.project;
    setPending(null);
    let failures = 0;
    for (const run of targets) {
      try {
        if (!(await deleteRun(run.id))) failures += 1;
      } catch {
        failures += 1;
      }
    }
    setDeleteError(
      failures ? `Could not delete ${failures} of ${targets.length} ${targets.length === 1 ? 'run' : 'runs'}.` : '',
    );
    if (project && !failures) changeHidden(project, false);
    await reload();
    // The row and the button that opened the dialog are gone: keep focus on the board.
    document.querySelector('main')?.focus();
  };
  let text = 'Loading runs…';
  if (failed) text = 'Unable to load runs. Check that the Jidoka server is running.';
  else if (deleteError) text = deleteError;
  else if (runs) {
    if (!runs.length) text = '';
    else if (!visibleRuns.length) text = 'All projects are hidden.';
    else text = '';
  }
  const indexed = (visibleRuns ?? []).map((run, index) => ({ run, index }));
  // Blocked Runs first, then the Project the page was opened on (`?project=`), then the most recent.
  const blockedFirst = (a, b) => Number(needsYou(b.run)) - Number(needsYou(a.run));
  const first = (run) => Number(projectOf(run) === wanted);
  const byPriority = (a, b) => blockedFirst(a, b) || first(b.run) - first(a.run) || compare(a, b);
  const line = indexed
    .filter((entry) => entry.run.status !== 'done' && (!onlyWaiting || needsYou(entry.run)))
    .sort(byPriority);
  // Newest first, so the day groups run from today backwards.
  const shipped = onlyWaiting ? [] : indexed.filter((entry) => entry.run.status === 'done').sort(compare);
  const projects = [...new Set([...(runs ?? []).map(projectOf), ...hiddenProjects])]
    .filter((path) => typeof path === 'string')
    .sort(
      (a, b) =>
        String(names.get(a) ?? a).localeCompare(String(names.get(b) ?? b)) || String(a).localeCompare(String(b)),
    );
  const counts = new Map();
  for (const run of runs ?? []) counts.set(projectOf(run), (counts.get(projectOf(run)) ?? 0) + 1);
  // The open detail is a Run id; it is looked up in the latest full list, so refreshes update it.
  const [openKey, setOpenKey] = useState(null);
  const lastOpened = useRef(null);
  // The control that opened the detail (a row title or a Run in the andon strip), to give focus back to.
  const opener = useRef(null);
  const openRun = openKey === null ? null : (runs ?? []).find((run) => runKey(run) === openKey);
  const open = (key) => {
    lastOpened.current = key;
    opener.current = document.activeElement;
    setOpenKey(key);
  };
  const close = () => setOpenKey(null);
  useEffect(() => {
    // The Run is no longer listed: close the detail, so it does not come back if the Run returns.
    if (openKey !== null && runs && !openRun) setOpenKey(null);
  }, [openKey, runs]);
  useEffect(() => {
    // Back to what opened the detail, once it is gone: the same control, else another for the same Run.
    if (openKey === null && lastOpened.current !== null) {
      const same = opener.current?.isConnected ? opener.current : null;
      const key = CSS.escape(String(lastOpened.current));
      const target =
        same ??
        document.querySelector(`.line [data-run-key="${key}"], .shipped [data-run-key="${key}"]`) ??
        document.querySelector(`[data-run-key="${key}"]`);
      target?.focus();
      // The control is gone (its Run left): keep focus on the board.
      if (!target || document.activeElement !== target) document.querySelector('main')?.focus();
      lastOpened.current = null;
      opener.current = null;
    }
  }, [openKey]);
  // The sticky top's height (--top-height), so the page scrolls a focused control below it into view, and its
  // toolbar's height (--toolbar-height), which scrolls away on narrow screens.
  const top = useRef(null);
  useEffect(() => {
    const element = top.current;
    const toolbar = element?.querySelector('.toolbar');
    if (!element || !toolbar || typeof ResizeObserver !== 'function') return;
    const style = document.documentElement.style;
    const observer = new ResizeObserver(() => {
      style.setProperty('--top-height', `${element.offsetHeight}px`);
      style.setProperty('--toolbar-height', `${toolbar.offsetHeight}px`);
    });
    observer.observe(element);
    observer.observe(toolbar);
    return () => observer.disconnect();
  }, []);
  return html`<${ProjectNames.Provider} value=${names}>
    <main tabindex="-1">
      <div class="top" ref=${top}>
        <div class="toolbar">
          <${NotificationsToggle} state=${notifications.state} onToggle=${notifications.toggle} />
          <button type="button" class="toggle help-open" onClick=${() => setHelpOpen(true)}>Help</button>
          ${projects.length
            ? html`<${ProjectsMenu}
                projects=${projects}
                hiddenProjects=${hiddenProjects}
                counts=${counts}
                onHide=${changeHidden}
                onDelete=${(project) => setPending({ project, runs: runs.filter((run) => projectOf(run) === project) })}
              />`
            : null}
          <${Connection} failed=${failed} lastOk=${lastOk} now=${now} />
        </div>
        ${visibleRuns
          ? html`<${Andon}
            waiting=${visibleRuns
              .map((run, index) => ({ run, index }))
              .filter((entry) => needsYou(entry.run))
              .sort(compare)
              .map((entry) => entry.run)}
            active=${visibleRuns.filter((run) => run.status !== 'done').length}
            filtered=${onlyWaiting}
            onFilter=${() => setOnlyWaiting((current) => !current)}
            onOpen=${open}
          />`
          : null}
        <p id="message" role="status" class=${failed || deleteError ? 'error' : ''}>${text}</p>
        ${visibleRuns?.length
          ? html`<section class="line" aria-labelledby="line-title">
              <div class="line-head">
                <h2 id="line-title">On the line<span class="count">${` (${line.length})`}</span></h2>
                <ol class="stations" aria-hidden="true">
                  ${STATIONS.map((id, at) => html`<li key=${id}>${GROUPS[at][1]}</li>`)}
                </ol>
              </div>
              ${line.length
                ? html`<ul class="line-rows">
                    ${line.map((entry) => html`<${LineRow} key=${runKey(entry.run)} run=${entry.run} now=${now} onOpen=${open} />`)}
                  </ul>`
                : html`<p class="empty">${onlyWaiting ? 'Nothing needs you right now.' : 'Nothing on the line.'}</p>`}
            </section>`
          : null}
      </div>
      ${runs && !runs.length ? html`<${Welcome} onHelp=${() => setHelpOpen(true)} />` : null}
      ${shipped.length
        ? html`<${ShippedLog}
            entries=${shipped}
            now=${now}
            onOpen=${open}
            onClear=${() => setPending({ shipped: true, runs: shipped.map((entry) => entry.run) })}
          />`
        : null}
      ${helpOpen ? html`<${Help} onClose=${closeHelp} reworkCap=${REWORK_CAP} />` : null}
      ${pending
        ? html`<${Confirm}
            title=${pending.project
              ? `Delete ${names.get(pending.project) ?? pending.project}?`
              : pending.shipped
                ? `Clear ${pending.runs.length} shipped ${pending.runs.length === 1 ? 'run' : 'runs'}?`
                : 'Delete this run?'}
            confirmLabel=${pending.shipped ? 'Clear shipped' : 'Delete'}
            onConfirm=${confirmDelete}
            onCancel=${() => setPending(null)}
          >
            <p>
              ${pending.project
                ? `This removes all ${pending.runs.length} run records for this project from the dashboard.`
                : pending.shipped
                  ? `This removes the records of the shipped runs from the dashboard.`
                  : html`This removes the record of <strong>${pending.runs[0].title}</strong> from the dashboard.`}
              The repository, worktrees, branches and pull requests stay as they are.
            </p>
          <//>`
        : null}
      ${openRun
        ? html`<${Detail}
            key=${runKey(openRun)}
            run=${openRun}
            now=${now}
            onClose=${close}
            onDelete=${(run) => setPending({ runs: [run] })}
            onSynced=${reload}
          />`
        : null}
    </main>
  <//>`;
}

render(html`<${App} />`, document.querySelector('#app'));
// The theme is a set-once preference, so it sits in the footer, next to the version.
render(html`<${ThemeSwitch} />`, document.querySelector('#theme-switch'));
