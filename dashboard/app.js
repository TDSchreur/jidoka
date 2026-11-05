import { createContext, h, render } from 'preact';
import { useContext, useEffect, useRef, useState } from 'preact/hooks';
import htm from 'htm';

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

/** Ticket progress as a chip, and the pull request and spec as links: the context a card shows. */
function ContextChips({ run }) {
  const progress = ticketProgress(run);
  return progress
    ? html`<span class="chip" title="Tickets done">Tickets${' '}${progress.done}/${progress.total}</span>`
    : null;
}

function ContextLinks({ run }) {
  const pr = run.pr ? html`<${RefLink} value=${run.pr} label="Pull request" />` : null;
  const spec = run.spec ? html`<${RefLink} value=${run.spec} label="Spec" />` : null;
  return pr || spec ? html`<p class="links">${pr}${pr && spec ? ' · ' : null}${spec}</p>` : null;
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
        <p class="project"><${ProjectName} path=${projectOf(run)} /></p>
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

function Card({ run, now, onOpen }) {
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
      <p class="project"><${ProjectName} path=${projectOf(run)} /></p>
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
  const summaries = GROUPS.filter(([id]) => run.summaries?.[id]).map(([id, name]) => [id, name]);
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
        <${Field} label="Mode" missing="Not recorded">${run.mode}<//>
        <${Field} label="Branch" missing="Not recorded">${run.branch}<//>
        <${Field} label="Worktree" missing="Not recorded">${run.worktree}<//>
        <${Field} label="Repository" missing="Not recorded">${run.repository}<//>
        <${Field} label="Base commit" missing="Not recorded">${run.base_commit}<//>
        <${Field} label="Spec" missing="Not recorded">
          ${run.spec ? html`<${TicketLink} value=${String(run.spec)} />` : null}
        <//>
        <${Field} label="Pull request" missing="Not recorded">
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
              : html`<${Card} key=${runKey(entry.run)} run=${entry.run} now=${now} onOpen=${onOpen} />`,
          )}
        </ul>`
      : html`<p class="empty">No runs</p>`}
  </section>`;
}

/** What the dashboard's words mean. A native modal: focus moves into it, and back to what opened it on close. */
function Help({ onClose }) {
  const dialog = useRef(null);
  const closing = useRef(onClose);
  closing.current = onClose;
  useEffect(() => {
    const element = dialog.current;
    if (!element.open) element.showModal();
    const onNativeClose = () => closing.current();
    element.addEventListener('close', onNativeClose);
    return () => element.removeEventListener('close', onNativeClose);
  }, []);
  return html`<dialog ref=${dialog} class="confirm help" aria-labelledby="help-title">
    <h2 id="help-title">Help</h2>
    <p>
      A <strong>Run</strong> is one piece of work carried through the factory flow. Each Run sits in the column of its
      Stage, or in Done once it has finished.
    </p>
    <h3>Stages</h3>
    <dl class="help-terms">
      ${GROUPS.map(([id, name, description]) => html`<dt key=${id}>${name}</dt><dd>${description}</dd>`)}
    </dl>
    <h3>Needs you</h3>
    <p>
      A Run that is blocked is waiting for you. Its card says <strong>Needs you</strong> and why, and it comes first in
      its column. The <em>Needs you</em> button shows only these Runs.
    </p>
    <h3>Rework</h3>
    <p>
      A <strong>Rework</strong> is a return from Check to Build to fix findings, followed by another Check. A Run gets
      at most ${REWORK_CAP} Reworks; after that you decide whether to do another, finish with the findings listed in the
      pull request, or stop. Check findings never block a Run.
    </p>
    <div class="confirm-buttons">
      <button type="button" class="toggle" autofocus onClick=${() => dialog.current.close()}>Close</button>
    </div>
  </dialog>`;
}

/** Shown instead of the columns while there are no Runs. */
function Welcome({ onHelp }) {
  return html`<section class="welcome" aria-labelledby="welcome-title">
    <h2 id="welcome-title">Welcome to the factory</h2>
    <p>
      No Runs yet. In your project, ask your coding agent to run <code>/factory</code>, <code>/factory-balanced</code>
      or <code>/factory-fast</code> with your task. Its Run appears here and follows the work from Define to a pull
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
function useNotifications(runs, names) {
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
  const waiting = runs ? runs.filter(needsYou).length : 0;
  const names = projectNames(runs ?? []);
  const notifications = useNotifications(runs, names);
  useEffect(() => {
    document.title = waiting ? `(${waiting}) ${TITLE}` : TITLE;
  }, [waiting]);
  // What the visitor is asked to confirm deleting: one Run, or every Done Run.
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
    await reload();
    // The card and the button that opened the dialog are gone: keep focus on the board.
    document.querySelector('main')?.focus();
  };
  let text = 'Loading runs…';
  if (failed) text = 'Unable to load runs. Check that the factory server is running.';
  else if (deleteError) text = deleteError;
  else if (runs) {
    if (!runs.length) text = '';
    else if (onlyWaiting && !waiting) text = 'Nothing needs you right now.';
    else text = '';
  }
  const indexed = (runs ?? [])
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
  if (opening.current === null && runs?.length) opening.current = firstStage(runs);
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
          <h2 class="project-heading">
            <button
              type="button"
              class="project-toggle"
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
      ${helpOpen ? html`<${Help} onClose=${closeHelp} />` : null}
      ${pending
        ? html`<${Confirm}
            title=${pending.runs.length === 1 ? 'Delete this run?' : `Delete all ${pending.runs.length} done runs in ${names.get(projectOf(pending.runs[0])) ?? projectOf(pending.runs[0])}?`}
            confirmLabel=${pending.runs.length === 1 ? 'Delete' : 'Delete all'}
            onConfirm=${confirmDelete}
            onCancel=${() => setPending(null)}
          >
            <p>
              ${pending.runs.length === 1
                ? html`This removes the record of <strong>${pending.runs[0].title}</strong> from the dashboard.`
                : `This removes the records of these runs in ${names.get(projectOf(pending.runs[0])) ?? projectOf(pending.runs[0])} from the dashboard.`}
              The worktree, branch and pull request stay as they are.
            </p>
          <//>`
        : null}
      ${openRun ? html`<${Detail} key=${runKey(openRun)} run=${openRun} now=${now} onClose=${close} />` : null}
    </main>
  <//>`;
}

render(html`<${App} />`, document.querySelector('#app'));
