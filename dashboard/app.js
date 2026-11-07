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
const REWORK_CAP = 2;
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

/** The ticket as a link when it is an http(s) URL, else as plain text. */
function TicketLink({ value }) {
  const url = httpUrl(value);
  return url ? html`<a href=${url.href} target="_blank" rel="noopener noreferrer">${value}</a>` : value;
}

/** The tickets listed on a Run, with how many are done; null when none are recorded. */
const ticketProgress = (run) => {
  const tickets = Array.isArray(run.tickets) ? run.tickets : [];
  return tickets.length ? { done: tickets.filter((t) => t.status === 'done').length, total: tickets.length } : null;
};

/** A link for a stored reference, only when it is an http(s) URL; nothing otherwise. */
function RefLink({ value, label }) {
  const url = httpUrl(value);
  return url ? html`<a href=${url.href} target="_blank" rel="noopener noreferrer">${label}</a>` : null;
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
  const noData = Array.isArray(usage.no_data) ? usage.no_data : [];
  const notes = Array.isArray(usage.notes) ? usage.notes : [];
  return html`<p>${status}</p>
    ${rows.length
      ? html`<div class="table-scroll" role="region" aria-label="Token usage by stage, model and ticket" tabindex="0">
            <table class="usage-table">
              <caption class="visually-hidden">Token usage by stage, model and ticket</caption>
              <thead>
                <tr>
                  <th scope="col">Stage</th>
                  <th scope="col">Model</th>
                  <th scope="col">Ticket</th>
                  <th scope="col">Fresh</th>
                  <th scope="col">Input</th>
                  <th scope="col">Cache read</th>
                  <th scope="col">Cache write</th>
                  <th scope="col">Output</th>
                  <th scope="col">Reasoning</th>
                  <th scope="col">Subagents</th>
                </tr>
              </thead>
              <tbody>
                ${rows.map(
                  (row, index) => html`<tr key=${index}>
                    <td>${stageName(row.stage)}</td>
                    <td>${row.model}</td>
                    <td><${TicketRef} value=${rowBucket(row)} /></td>
                    <td>${tokenCount(freshTokens(row))}</td>
                    <td>${tokenCount(row.input)}</td>
                    <td>${tokenCount(row.cache_read)}</td>
                    <td>${tokenCount(row.cache_write)}</td>
                    <td>${tokenCount(row.output)}</td>
                    <td>${tokenCount(row.reasoning)}</td>
                    <td>${row.subagents ? tokenCount(freshTokens(row.subagents)) : '-'}</td>
                  </tr>`,
                )}
              </tbody>
              <tfoot>
                <tr>
                  <th scope="row" colspan="3">Total fresh</th>
                  <td>${tokenCount(total)}</td>
                  <td colspan="6"></td>
                </tr>
              </tfoot>
            </table>
          </div>
          <p class="usage-note">
            Fresh tokens are input excluding cache reads, plus cache writes and output. Cache reads are shown beside
            them, not inside. Reasoning is part of output. n/a means the log did not report it; it is not zero.
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

/** The pull request state for the detail: the state and when it was read, or why it is not known. */
function PrStateDetail({ run, now }) {
  const status = run.pr_status;
  if (!run.pr) return null;
  if (!status || status.url !== run.pr) return html`<span class="not-set">Not read yet</span>`;
  if (status.state === 'unknown') return html`Unknown: ${status.reason ?? 'no reason given'}`;
  return html`<${PrStateLabel} run=${run} />${' '}<span class="updated">checked ${ago(now - status.checked_at)}</span>`;
}

function ContextLinks({ run }) {
  const named = (label) => html`${label}<span class="visually-hidden">${` of ${run.title}`}</span>`;
  const pr = run.pr ? html`<${RefLink} value=${run.pr} label=${named('Pull request')} />` : null;
  const spec = run.spec ? html`<${RefLink} value=${run.spec} label=${named('Spec')} />` : null;
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
 * One labelled value of the Run detail; an empty value reads "Not set". `wide` puts the label above the value and
 * gives the value the whole width, for the token table.
 */
function Field({ label, children, missing = 'Not set', wide = false }) {
  const empty = children === null || children === undefined || children === '';
  const classes = [wide ? 'wide' : null, empty ? 'not-set' : null].filter(Boolean).join(' ') || null;
  return html`<dt class=${wide ? 'wide' : null}>${label}</dt>
    <dd class=${classes}>${empty ? missing : children}</dd>`;
}

/** Every field of a Run in a native modal dialog on the right. The browser handles focus containment and Escape. */
function Detail({ run, now, onClose, onDelete }) {
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
  const progress = ticketProgress(run);
  const summaries = GROUPS.filter(([id]) => run.summaries?.[id]);
  const stage = GROUPS.find(([id]) => id === run.stage);
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
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
        <button type="button" class="text-button detail-delete" onClick=${() => onDelete(run)}>
          Delete run<span class="visually-hidden">${` ${run.title}`}</span>
        </button>
        <button type="button" class="toggle detail-close" onClick=${dismiss}>
          <span class="detail-close-wide">Close</span><span class="detail-close-compact">Back</span>
        </button>
      </div>
      <h2 id="detail-title">${run.title}</h2>
      <dl class="fields">
        <${Field} label="Project"><${ProjectName} path=${projectOf(run)} /><//>
        <${Field} label="Stage">${stage ? stage[1] : String(run.stage)}<//>
        <${Field} label="Status">
          <span class=${STATUSES.includes(run.status) ? `badge ${run.status}` : 'badge'}
            >${statusLabel(run.status)}</span
          >
        <//>
        <${Field} label="Activity">${run.activity}<//>
        ${run.current_ticket
          ? html`<${Field} label="Current ticket"><${TicketLink} value=${String(run.current_ticket)} /><//>`
          : null}
        ${run.blocking_reason ? html`<${Field} label="Status reason">${run.blocking_reason}<//>` : null}
        <${Field} label="Mode" missing="Unknown">${run.mode}<//>
        <${Field} label="Branch" missing="Unknown">${run.branch}<//>
        <${Field} label="Worktree" missing="Unknown">${run.worktree}<//>
        <${Field} label="Repository" missing="Unknown">${run.repository}<//>
        <${Field} label="Base commit" missing="Unknown">${run.base_commit}<//>
        <${Field} label="Spec" missing="Unknown">
          ${run.spec ? html`<${TicketLink} value=${String(run.spec)} />` : null}
        <//>
        <${Field} label="Pull request" missing="Unknown">
          ${run.pr ? html`<${TicketLink} value=${String(run.pr)} />` : null}
        <//>
        <${Field} label="Pull request state" missing="No pull request">
          ${run.pr ? html`<${PrStateDetail} run=${run} now=${now} />` : null}
        <//>
        <${Field} label=${progress ? `Tickets (${progress.done} of ${progress.total} done)` : 'Tickets'} missing="None recorded">
          ${progress
            ? html`<ol class="ticket-list">
                ${run.tickets.map(
                  (ticket) => html`<li key=${ticket.reference}>
                    <span class="ticket-status">${TICKET_STATUS[ticket.status] ?? String(ticket.status)}</span>${' '}
                    <${TicketLink} value=${String(ticket.reference)} />${' '}${ticket.title}
                    <p>Agent-reported; entries may be incomplete.</p>
                    ${Array.isArray(run.ticket_reports) && run.ticket_reports.some((report) => report.ticket === ticket.reference)
                      ? html`<ul>${run.ticket_reports.filter((report) => report.ticket === ticket.reference).map((report) => html`<li>
                          <strong>${report.kind === 'skill' ? 'Skill' : 'Activity'}:</strong>${' '}${report.text}${' '}
                          (${report.stage}, <time datetime=${new Date(report.at).toISOString()}>${localDateTime(new Date(report.at))}</time>)
                        </li>`)}</ul>`
                      : html`<p>No reports recorded.</p>`}
                  </li>`,
                )}
              </ol>`
            : null}
        <//>
        <${Field} label="Run reports" wide>
          <p>Agent-reported; entries may be incomplete.</p>
          ${Array.isArray(run.run_reports) && run.run_reports.length
            ? html`<ul>${run.run_reports.map((report) => html`<li>
                <strong>${report.kind === 'skill' ? 'Skill' : 'Activity'}:</strong>${' '}${report.text}${' '}
                (${report.stage}, <time datetime=${new Date(report.at).toISOString()}>${localDateTime(new Date(report.at))}</time>)
              </li>`)}</ul>`
            : html`<p>No reports recorded.</p>`}
        <//>
        <${Field} label="Token usage" missing="Not tracked" wide>
          ${run.usage ? html`<${UsageDetail} usage=${run.usage} />` : null}
        <//>
        <${Field} label="Stage summaries" missing="None recorded">
          ${summaries.length
            ? html`<ul class="summaries">
                ${summaries.map(([id, name]) => html`<li key=${id}><strong>${name}:</strong>${' '}${run.summaries[id]}</li>`)}
              </ul>`
            : null}
        <//>
        <${Field} label="Rework">${rework !== null ? reworkLabel(rework) : null}<//>
        <${Field} label="Updated">
          ${ts !== null
            ? html`<time datetime=${new Date(ts).toISOString()}>${ago(now - ts)}</time>${' '}(${localDateTime(
                new Date(ts),
              )})`
            : null}
        <//>
        <${Field} label="Run id"><code>${run.id}</code><//>
      </dl>
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
    <h2 id="welcome-title">Welcome to the factory</h2>
    <p>
      No Runs yet. In your project, ask your coding agent to run <code>/factory</code> to select a workflow, or choose
      <code>/factory-full</code>, <code>/factory-balanced</code>, or <code>/factory-fast</code> directly. Its Run appears here and follows the work from Define to a pull
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

const TITLE = 'TDS Factory';
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
  if (failed) text = 'Unable to load runs. Check that the factory server is running.';
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
          />`
        : null}
    </main>
  <//>`;
}

render(html`<${App} />`, document.querySelector('#app'));
// The theme is a set-once preference, so it sits in the footer, next to the version.
render(html`<${ThemeSwitch} />`, document.querySelector('#theme-switch'));
