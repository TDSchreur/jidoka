import { createContext, h, render } from 'preact';
import { useContext, useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import htm from 'htm';
import { Help } from '/assets/help.js';

const html = htm.bind(h);

const GROUPS = [
  ['define', 'Define', 'Clarify the work, write a spec and create tickets.'],
  ['build', 'Build', 'Implement the spec and run the relevant checks.'],
  ['check', 'Check', 'Review the code and scan for security issues.'],
  ['finish', 'Finish', 'Create a pull request for the completed work.'],
  ['done', 'Done', 'Runs that have finished, whatever their stage.'],
];
const STATUSES = ['running', 'blocked', 'done'];
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

/** Project paths the visitor collapsed. Blocked or broken storage only means the choice is not remembered. */
function storedCollapsed() {
  try {
    const value = JSON.parse(localStorage.getItem('collapsed-projects'));
    return new Set(Array.isArray(value) ? value.filter((path) => typeof path === 'string') : []);
  } catch {
    return new Set();
  }
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
const groupOf = (run) => (run.status === 'done' ? 'done' : run.stage);

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

/** The ticket as a link when it is an http(s) URL, else as plain text. */
function TicketLink({ value }) {
  let url = null;
  try {
    const parsed = new URL(value);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') url = parsed;
  } catch {
    /* not a URL: show as text */
  }
  return url ? html`<a href=${url.href} rel="noopener noreferrer">${value}</a>` : value;
}

/** The tickets listed on a Run, with how many are done; null when none are recorded. */
const ticketProgress = (run) => {
  const tickets = Array.isArray(run.tickets) ? run.tickets : [];
  return tickets.length ? { done: tickets.filter((t) => t.status === 'done').length, total: tickets.length } : null;
};

/** A link for a stored reference, only when it is an http(s) URL. */
function RefLink({ value, label }) {
  try {
    const url = new URL(String(value));
    if (url.protocol === 'http:' || url.protocol === 'https:')
      return html`<a href=${url.href} rel="noopener noreferrer">${label}</a>`;
  } catch {
    /* not a URL: no link on a card */
  }
  return null;
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

/** Ticket progress and fresh tokens as chips, and the pull request and spec as links: the context a card shows. */
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
                    <td>${rowBucket(row)}</td>
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

function ContextLinks({ run }) {
  const named = (label) => html`${label}<span class="visually-hidden">${` of ${run.title}`}</span>`;
  const pr = run.pr ? html`<${RefLink} value=${run.pr} label=${named('Pull request')} />` : null;
  const spec = run.spec ? html`<${RefLink} value=${run.spec} label=${named('Spec')} />` : null;
  return pr || spec
    ? html`<p class="links">${pr}${pr && spec ? html`<span aria-hidden="true">${' · '}</span>` : null}${spec}</p>`
    : null;
}

function Ticket({ value }) {
  return html`<p class="ticket">Ticket: <${TicketLink} value=${value} /></p>`;
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
  if (ts === null) return html`<p class="updated">Updated: not reported</p>`;
  const date = new Date(ts);
  return html`<p class="updated">
    Updated${' '}<time datetime=${date.toISOString()} title=${date.toLocaleString()}
      >${ago(now - ts)}</time
    >
  </p>`;
}

/** A Done card is one line: the title, a Show details toggle and Delete. The rest shows on demand. */
function DoneCard({ run, now, onOpen, onDelete }) {
  const [expanded, setExpanded] = useState(false);
  const detailsId = `done-details-${runKey(run).replace(/\W/g, '-')}`;
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  return html`<li>
    <article class="run done-run">
      <div class="done-line">
        <h3>
          <button type="button" class="card-open" data-run-key=${runKey(run)} onClick=${() => onOpen(runKey(run))}>
            ${run.title}
          </button>
        </h3>
        <button
          type="button"
          class="toggle"
          aria-expanded=${String(expanded)}
          aria-controls=${detailsId}
          onClick=${() => setExpanded((current) => !current)}
        >
          ${expanded ? 'Hide details' : 'Show details'}<span class="visually-hidden">${` for ${run.title}`}</span>
        </button>
        <button type="button" class="toggle danger" onClick=${() => onDelete(run)}>
          Delete<span class="visually-hidden">${` ${run.title}`}</span>
        </button>
      </div>
      <div id=${detailsId} hidden=${!expanded}>
        <div class="chips">
          <span class="badge done">${String(run.status)}</span>
          ${rework !== null
            ? html`<span class="chip">Rework${' '}${reworkLabel(rework)}</span>`
            : null}
          <${ContextChips} run=${run} />
        </div>
        <${ContextLinks} run=${run} />
        ${run.activity ? html`<p class="activity">Activity: ${run.activity}</p>` : null}
        ${run.current_ticket ? html`<${Ticket} value=${String(run.current_ticket)} />` : null}
        <${Updated} run=${run} now=${now} />
      </div>
    </article>
  </li>`;
}

function Card({ run, now, onOpen, onDelete }) {
  const known = STATUSES.includes(run.status);
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  const blocked = run.status === 'blocked';
  return html`<li>
    <article class=${blocked ? 'run blocked' : 'run'}>
      ${blocked
        ? html`<p class="needs-you">
            <span class="needs-you-label">Needs you</span>
            ${run.blocking_reason ? ` ${run.blocking_reason}` : null}
          </p>`
        : null}
      <h3>
        <button type="button" class="card-open" data-run-key=${runKey(run)} onClick=${() => onOpen(runKey(run))}>
          ${run.title}
        </button>
      </h3>
      <div class="chips">
        <span class=${known ? `badge ${run.status}` : 'badge'}>${String(run.status)}</span>
        ${rework !== null
          ? html`<span class="chip"
              >Rework${' '}${reworkLabel(rework)}</span
            >`
          : null}
        <${ContextChips} run=${run} />
      </div>
      <${ContextLinks} run=${run} />
      ${run.activity ? html`<p class="activity">Activity: ${run.activity}</p>` : null}
      ${run.current_ticket ? html`<${Ticket} value=${String(run.current_ticket)} />` : null}
      <${Updated} run=${run} now=${now} />
      <button type="button" class="toggle danger" onClick=${() => onDelete(run)}>
        Delete<span class="visually-hidden">${` ${run.title}`}</span>
      </button>
    </article>
  </li>`;
}

/** One labelled value of the Run detail; an empty value reads "Not set". */
function Field({ label, children, missing = 'Not set' }) {
  const empty = children === null || children === undefined || children === '';
  return html`<dt>${label}</dt>
    <dd class=${empty ? 'not-set' : null}>${empty ? missing : children}</dd>`;
}

/** Every field of a Run in a native modal dialog on the right. The browser handles focus containment and Escape. */
function Detail({ run, now, onClose }) {
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
            >${String(run.status)}</span
          >
        <//>
        <${Field} label="Activity">${run.activity}<//>
        <${Field} label="Ticket">
          ${run.current_ticket ? html`<${TicketLink} value=${String(run.current_ticket)} />` : null}
        <//>
        <${Field} label="Status reason">${run.blocking_reason}<//>
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
        <${Field} label=${progress ? `Tickets (${progress.done} of ${progress.total} done)` : 'Tickets'} missing="None recorded">
          ${progress
            ? html`<ol class="ticket-list">
                ${run.tickets.map(
                  (ticket) => html`<li key=${ticket.reference}>
                    <span class="ticket-status">${TICKET_STATUS[ticket.status] ?? String(ticket.status)}</span>${' '}
                    <${TicketLink} value=${String(ticket.reference)} />${' '}${ticket.title}
                  </li>`,
                )}
              </ol>`
            : null}
        <//>
        <${Field} label="Token usage" missing="Not tracked">
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
            ? html`<time datetime=${new Date(ts).toISOString()}>${ago(now - ts)}</time>${' '}(${new Date(
                ts,
              ).toLocaleString()})`
            : null}
        <//>
        <${Field} label="Run id"><code>${run.id}</code><//>
      </dl>
    </div>
  </dialog>`;
}

function Group({ id, entries, now, selected, onOpen, onDelete, onDeleteAll }) {
  return html`<section class=${selected ? 'group selected' : 'group'} aria-label=${GROUPS.find(([key]) => key === id)[1]}>
    ${id === 'done' && entries.length
      ? html`<button type="button" class="toggle danger delete-all" onClick=${onDeleteAll}>
          Delete all<span class="visually-hidden">${` ${entries.length} done runs`}</span>
        </button>`
      : null}
    ${entries.length
      ? html`<ul class="runs">
          ${entries.map((entry) =>
            id === 'done'
              ? html`<${DoneCard} key=${runKey(entry.run)} run=${entry.run} now=${now} onOpen=${onOpen} onDelete=${onDelete} />`
              : html`<${Card} key=${runKey(entry.run)} run=${entry.run} now=${now} onOpen=${onOpen} onDelete=${onDelete} />`,
          )}
        </ul>`
      : html`<p class="empty">No runs</p>`}
  </section>`;
}

/** Shown instead of the columns while there are no Runs. */
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
  const [state, setState] = useState({ runs: null, failed: false, lastOk: null, beat: 0 });
  const reload = useRef(() => {});
  useEffect(() => {
    let active = true;
    async function refresh() {
      try {
        const response = await fetch('/api/runs', { cache: 'no-store' });
        if (!response.ok) throw new Error('Failed to load runs');
        const data = await response.json();
        if (!Array.isArray(data)) throw new Error('Unexpected response');
        if (active) setState((current) => ({ runs: data, failed: false, lastOk: Date.now(), beat: current.beat + 1 }));
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

/** The current time, refreshed so relative times keep counting. */
/** Whether the page can reach the server: a dot that pulses on every answer, with the state in words. */
function Connection({ failed, lastOk, beat, now }) {
  const state = failed ? 'offline' : lastOk === null ? 'connecting' : 'connected';
  const label = { offline: 'Offline', connecting: 'Connecting…', connected: 'Connected' }[state];
  const age =
    lastOk === null
      ? ''
      : state === 'offline'
        ? `last contact ${ago(now - lastOk)}`
        : `updated ${ago(now - lastOk)}`;
  return html`<p class=${`connection ${state}`}>
    <span class="pulse" key=${beat} aria-hidden="true"></span>
    <span role="status">${label}</span>${age ? html`<span class="connection-age">${` · ${age}`}</span>` : null}
  </p>`;
}

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

function NeedsYouToggle({ count, pressed, onToggle }) {
  return html`<button type="button" class="toggle" aria-pressed=${String(pressed)} onClick=${onToggle}>
    Needs you (${count})
  </button>`;
}

/** The Stage a visitor first sees in compact mode: one with a waiting Run, else the first with any Run. */
function firstStage(runs) {
  const waiting = runs.find(needsYou);
  if (waiting) return groupOf(waiting);
  return (GROUPS.find(([id]) => runs.some((run) => groupOf(run) === id)) ?? GROUPS[0])[0];
}

/** Compact mode shows one Stage at a time; these buttons pick it. Wide screens hide them and show every column. */
function StageButtons({ stages, current, onPick }) {
  return html`<div class="stages" role="group" aria-label="Stage">
    ${stages.map((stage) => {
      const waiting = stage.entries.some((entry) => needsYou(entry.run));
      return html`<button
        type="button"
        key=${stage.id}
        class="toggle"
        aria-pressed=${String(stage.id === current)}
        onClick=${() => onPick(stage.id)}
      >
        ${stage.name} (${stage.entries.length})${waiting
          ? html`<span class="waiting-mark" aria-hidden="true">!</span
              ><span class="visually-hidden"> needs you</span>`
          : null}
      </button>`;
    })}
  </div>`;
}

function App() {
  const { runs, failed, lastOk, beat, reload } = useRuns();
  const now = useNow();
  const [onlyWaiting, setOnlyWaiting] = useState(false);
  const [wanted] = useState(requestedProject);
  // The requested Project is shown expanded for this visit, without touching what was remembered.
  const [collapsed, setCollapsed] = useState(storedCollapsed);
  const [forcedOpen, setForcedOpen] = useState(() => new Set(wanted === null ? [] : [wanted]));
  const isCollapsed = (project) => collapsed.has(project) && !forcedOpen.has(project);
  const toggleProject = (project) => {
    const next = new Set(collapsed);
    if (isCollapsed(project)) next.delete(project);
    else next.add(project);
    setCollapsed(next);
    setForcedOpen((current) => new Set([...current].filter((path) => path !== project)));
    try {
      localStorage.setItem('collapsed-projects', JSON.stringify([...next]));
    } catch {
      /* storage is blocked: the choice lasts until the page is reloaded */
    }
  };
  const [hiddenProjects, setHiddenProjects] = useState(storedHiddenProjects);
  const hiddenFocus = useRef(null);
  useLayoutEffect(() => {
    if (!hiddenFocus.current) return;
    const { project, hidden } = hiddenFocus.current;
    hiddenFocus.current = null;
    requestAnimationFrame(() => {
      const target = hidden
        ? document.querySelector('.hidden-projects summary')
        : document.querySelector(`[data-project-toggle="${CSS.escape(project)}"]`) ?? document.querySelector('.hidden-projects summary');
      (target ?? document.querySelector('.help-open'))?.focus();
    });
  }, [hiddenProjects]);
  const changeHidden = (project, hidden) => {
    const next = new Set(hiddenProjects);
    if (hidden) next.add(project);
    else next.delete(project);
    hiddenFocus.current = { project, hidden };
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
    // The opener may be gone (the welcome card left); fall back to the header button.
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
    // The card and the button that opened the dialog are gone: keep focus on the board.
    document.querySelector('main')?.focus();
  };
  let text = 'Loading runs…';
  if (failed) text = 'Unable to load runs. Check that the factory server is running.';
  else if (deleteError) text = deleteError;
  else if (runs) {
    if (!runs.length) text = '';
    else if (!visibleRuns.length) text = 'All projects are hidden.';
    else if (onlyWaiting && !waiting) text = 'Nothing needs you right now.';
    else text = '';
  }
  const indexed = (visibleRuns ?? [])
    .map((run, index) => ({ run, index }))
    .filter((entry) => !onlyWaiting || needsYou(entry.run));
  const stages = GROUPS.map(([id, name, description]) => ({
    id,
    name,
    description,
    entries: indexed.filter((entry) => groupOf(entry.run) === id).sort(compare),
  }));
  const projects = [...new Set(indexed.map((entry) => projectOf(entry.run)))].sort(
    (a, b) =>
      // The Project the page was opened on comes first, the rest alphabetically.
      Number(b === wanted) - Number(a === wanted) ||
      String(names.get(a) ?? a).localeCompare(String(names.get(b) ?? b)) ||
      String(a).localeCompare(String(b)),
  );
  // Chosen once, from the first answer that has Runs (whatever the filter), then only the visitor changes it.
  const [picked, setPicked] = useState(null);
  const opening = useRef(null);
  if (opening.current === null && visibleRuns?.length) opening.current = firstStage(visibleRuns);
  const current = picked ?? opening.current ?? GROUPS[0][0];
  // The open detail is a Run id; it is looked up in the latest full list, so refreshes update it.
  const [openKey, setOpenKey] = useState(null);
  const lastOpened = useRef(null);
  const openRun = openKey === null ? null : (runs ?? []).find((run) => runKey(run) === openKey);
  const open = (key) => {
    lastOpened.current = key;
    setOpenKey(key);
  };
  const close = () => setOpenKey(null);
  useEffect(() => {
    // The Run is no longer listed: close the detail, so it does not come back if the Run returns.
    if (openKey !== null && runs && !openRun) setOpenKey(null);
  }, [openKey, runs]);
  useEffect(() => {
    // Back to the card that was opened, once the detail is gone.
    if (openKey === null && lastOpened.current !== null) {
      const card = document.querySelector(`[data-run-key="${CSS.escape(String(lastOpened.current))}"]`);
      card?.focus();
      // The card is gone or hidden (its Run left, or changed Stage): keep focus on the board.
      if (!card || document.activeElement !== card) document.querySelector('main')?.focus();
      lastOpened.current = null;
    }
  }, [openKey]);
  return html`<${ProjectNames.Provider} value=${names}>
    <div class="toolbar">
      <${NeedsYouToggle}
        count=${waiting}
        pressed=${onlyWaiting}
        onToggle=${() => setOnlyWaiting((current) => !current)}
      />
      <${NotificationsToggle} state=${notifications.state} onToggle=${notifications.toggle} />
      <button type="button" class="toggle help-open" onClick=${() => setHelpOpen(true)}>Help</button>
      <${ThemeSwitch} />
      <${Connection} failed=${failed} lastOk=${lastOk} beat=${beat} now=${now} />
    </div>
    <main tabindex="-1">
      ${hiddenProjects.size ? html`<details class="hidden-projects">
        <summary>Hidden projects (${hiddenProjects.size})</summary>
        <p>Hidden projects are remembered in this browser. Their runs are kept.</p>
        <ul>${[...hiddenProjects].sort().map((project) => html`<li key=${project}><${ProjectName} path=${project} /> <button type="button" class="toggle" onClick=${() => changeHidden(project, false)} aria-label=${`Restore ${names.get(project) ?? project}`}>Restore</button> ${runs?.some((run) => projectOf(run) === project) ? html`<button type="button" class="toggle danger delete-project" onClick=${() => setPending({ project, runs: runs.filter((run) => projectOf(run) === project) })} aria-label=${`Delete ${names.get(project) ?? project}`}>Delete</button>` : null}</li>`)}</ul>
      </details>` : null}
      <p id="message" role="status" class=${failed || deleteError ? 'error' : ''}>${text}</p>
      ${runs && !runs.length ? html`<${Welcome} onHelp=${() => setHelpOpen(true)} />` : null}
      ${runs?.length ? html`<${StageButtons} stages=${stages} current=${current} onPick=${setPicked} />` : null}
      ${runs?.length
        ? html`<div class="board-headers">
            ${stages.map((stage) => html`<div class="group-header"><h2>${stage.name}<span class="count">${` (${stage.entries.length})`}</span></h2><p class="group-description">${stage.description}</p></div>`)}
          </div>`
        : null}
      ${runs?.length ? html`<p class="compact-description">${stages.find((stage) => stage.id === current).description}</p>` : null}
      <div id="groups">
        ${projects.map((project) => {
          const closed = isCollapsed(project);
          const groupsId = `project-groups-${projects.indexOf(project)}`;
          const projectWaiting = indexed.filter((entry) => projectOf(entry.run) === project && needsYou(entry.run)).length;
          return html`<section class="project-row" key=${project}>
          <div class="project-heading-row">
          <h2 class="project-heading">
            <button
              type="button"
              class="project-toggle"
              data-project-toggle=${project}
              aria-expanded=${String(!closed)}
              aria-controls=${groupsId}
              onClick=${() => toggleProject(project)}
            >
              <span class="project-chevron" aria-hidden="true">${closed ? '▸' : '▾'}</span
              ><${ProjectName} path=${project} />
            </button>
            ${closed
              ? html`<span class="project-summary"
                  >${stages
                    .map((item) => `${item.name} ${item.entries.filter((entry) => projectOf(entry.run) === project).length}`)
                    .join(' · ')}</span
                >${projectWaiting
                  ? html`<span class="badge blocked">${projectWaiting} waiting</span>`
                  : null}`
              : null}
          </h2>
          <button type="button" class="toggle danger delete-project" onClick=${() => setPending({ project, runs: runs.filter((run) => projectOf(run) === project) })} aria-label=${`Delete ${names.get(project) ?? project}`}>Delete</button>
          <button type="button" class="toggle hide-project" onClick=${() => changeHidden(project, true)} aria-label=${`Hide ${names.get(project) ?? project}`}>Hide</button>
          </div>
          <div class="project-groups" id=${groupsId} hidden=${closed}>
            ${stages.map((item) => {
              const entries = item.entries.filter((entry) => projectOf(entry.run) === project);
              return html`<${Group}
                key=${item.id}
                id=${item.id}
                now=${now}
                entries=${entries}
                selected=${item.id === current}
                onOpen=${open}
                onDelete=${(run) => setPending({ runs: [run] })}
                onDeleteAll=${() => setPending({ runs: entries.map((entry) => entry.run) })}
              />`;
            })}
          </div>
        </section>`;
        })}
      </div>
      ${helpOpen ? html`<${Help} onClose=${closeHelp} reworkCap=${REWORK_CAP} />` : null}
      ${pending
        ? html`<${Confirm}
            title=${pending.project ? `Delete ${names.get(pending.project) ?? pending.project}?` : pending.runs.length === 1 ? 'Delete this run?' : `Delete all ${pending.runs.length} done runs in ${names.get(projectOf(pending.runs[0])) ?? projectOf(pending.runs[0])}?`}
            confirmLabel=${pending.project || pending.runs.length === 1 ? 'Delete' : 'Delete all'}
            onConfirm=${confirmDelete}
            onCancel=${() => setPending(null)}
          >
            <p>
              ${pending.project
                ? `This removes all ${pending.runs.length} run records for this project from the dashboard.`
                : pending.runs.length === 1
                ? html`This removes the record of <strong>${pending.runs[0].title}</strong> from the dashboard.`
                : `This removes the records of these runs in ${names.get(projectOf(pending.runs[0])) ?? projectOf(pending.runs[0])} from the dashboard.`}
              The repository, worktrees, branches and pull requests stay as they are.
            </p>
          <//>`
        : null}
      ${openRun ? html`<${Detail} key=${runKey(openRun)} run=${openRun} now=${now} onClose=${close} />` : null}
    </main>
  <//>`;
}

render(html`<${App} />`, document.querySelector('#app'));
