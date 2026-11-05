import { h, render } from 'preact';
import { useEffect, useRef, useState } from 'preact/hooks';
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
const REWORK_CAP = 2;
const POLL_MS = 3000;
const NOW_MS = 30000;
const THEMES = [
  ['system', 'System'],
  ['light', 'Light'],
  ['dark', 'Dark'],
];

const timestamp = (run) =>
  typeof run.updated_at === 'number' &&
  Number.isFinite(run.updated_at) &&
  !Number.isNaN(new Date(run.updated_at).getTime())
    ? run.updated_at
    : null;
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
        <button type="button" class="card-open" data-run-id=${run.id} onClick=${() => onOpen(run.id)}>
          ${run.title}
        </button>
      </h3>
      <p class="project">${run.project}</p>
      <div class="chips">
        <span class=${known ? `badge ${run.status}` : 'badge'}>${String(run.status)}</span>
        ${rework !== null
          ? html`<span class="chip"
              >Rework${' '}${reworkLabel(rework)}</span
            >`
          : null}
      </div>
      ${run.activity ? html`<p class="activity">Activity: ${run.activity}</p>` : null}
      ${run.current_ticket ? html`<${Ticket} value=${String(run.current_ticket)} />` : null}
      <${Updated} run=${run} now=${now} />
    </article>
  </li>`;
}

/** One labelled value of the Run detail; an empty value reads "Not set". */
function Field({ label, children }) {
  const empty = children === null || children === undefined || children === '';
  return html`<dt>${label}</dt>
    <dd class=${empty ? 'not-set' : null}>${empty ? 'Not set' : children}</dd>`;
}

/** Every field of a Run in a native modal dialog on the right. The browser handles focus containment and Escape. */
function Detail({ run, now, onClose }) {
  const dialog = useRef(null);
  useEffect(() => {
    const element = dialog.current;
    if (!element.open) element.showModal();
    // Escape has no hook of its own: the browser closes the dialog and fires `close`.
    element.addEventListener('close', onClose);
    return () => element.removeEventListener('close', onClose);
  }, []);
  // Close the dialog and tell the page now, rather than waiting for the `close` event.
  const dismiss = () => {
    dialog.current.close();
    onClose();
  };
  const ts = timestamp(run);
  const stage = GROUPS.find(([id]) => id === run.stage);
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  return html`<dialog
    ref=${dialog}
    class="detail"
    aria-labelledby="detail-title"
    onClick=${(event) => {
      // A click on the backdrop lands on the dialog itself; the body below fills the rest of it.
      if (event.target === event.currentTarget) dismiss();
    }}
  >
    <div class="detail-body">
      <div class="detail-bar">
        <button type="button" class="toggle detail-close" onClick=${dismiss}>
          Close
        </button>
      </div>
      <h2 id="detail-title">${run.title}</h2>
      <dl class="fields">
        <${Field} label="Project">${run.project}<//>
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
        <${Field} label="Blocking reason">${run.blocking_reason}<//>
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

function Group({ id, name, description, entries, now, selected, onOpen }) {
  return html`<section class=${selected ? 'group selected' : 'group'} aria-labelledby=${`group-${id}`}>
    <div class="group-header">
      <h2 id=${`group-${id}`}>${name}<span class="count">${` (${entries.length})`}</span></h2>
      <p class="group-description">${description}</p>
    </div>
    ${entries.length
      ? html`<ul class="runs">
          ${entries.map((entry) => html`<${Card} key=${entry.index} run=${entry.run} now=${now} onOpen=${onOpen} />`)}
        </ul>`
      : html`<p class="empty">No runs</p>`}
  </section>`;
}

/** Polls the Run list; `runs` stays null until the first answer and `failed` is set while the server is unreachable. */
function useRuns() {
  const [state, setState] = useState({ runs: null, failed: false });
  useEffect(() => {
    let active = true;
    async function refresh() {
      try {
        const response = await fetch('/api/runs', { cache: 'no-store' });
        if (!response.ok) throw new Error('Failed to load runs');
        const data = await response.json();
        if (!Array.isArray(data)) throw new Error('Unexpected response');
        if (active) setState({ runs: data, failed: false });
      } catch {
        if (active) setState((current) => ({ ...current, failed: true }));
      }
    }
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, []);
  return state;
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
  const { runs, failed } = useRuns();
  const now = useNow();
  const [onlyWaiting, setOnlyWaiting] = useState(false);
  const waiting = runs ? runs.filter(needsYou).length : 0;
  useEffect(() => {
    document.title = waiting ? `(${waiting}) ${TITLE}` : TITLE;
  }, [waiting]);
  let text = 'Loading runs…';
  if (failed) text = 'Unable to load runs. Check that the factory server is running.';
  else if (runs) {
    if (!runs.length) text = 'No runs yet. Start one with the factory skill.';
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
  // Chosen once, from the first answer that has Runs (whatever the filter), then only the visitor changes it.
  const [picked, setPicked] = useState(null);
  const opening = useRef(null);
  if (opening.current === null && runs?.length) opening.current = firstStage(runs);
  const current = picked ?? opening.current ?? GROUPS[0][0];
  // The open detail is a Run id; it is looked up in the latest full list, so refreshes update it.
  const [openId, setOpenId] = useState(null);
  const lastOpened = useRef(null);
  const openRun = openId === null ? null : (runs ?? []).find((run) => run.id === openId);
  const open = (id) => {
    lastOpened.current = id;
    setOpenId(id);
  };
  const close = () => setOpenId(null);
  useEffect(() => {
    // Back to the card that was opened, once the detail is gone.
    if (openId === null && lastOpened.current !== null) {
      document.querySelector(`[data-run-id="${lastOpened.current}"]`)?.focus();
      lastOpened.current = null;
    }
  }, [openId]);
  return html`
    <div class="toolbar">
      <${NeedsYouToggle}
        count=${waiting}
        pressed=${onlyWaiting}
        onToggle=${() => setOnlyWaiting((current) => !current)}
      />
      <${ThemeSwitch} />
    </div>
    <main>
      <p id="message" role="status" class=${failed ? 'error' : ''}>${text}</p>
      ${runs ? html`<${StageButtons} stages=${stages} current=${current} onPick=${setPicked} />` : null}
      <div id="groups">
        ${runs
          ? stages.map(
              (item) => html`<${Group}
                key=${item.id}
                id=${item.id}
                name=${item.name}
                description=${item.description}
                now=${now}
                entries=${item.entries}
                selected=${item.id === current}
                onOpen=${open}
              />`,
            )
          : null}
      </div>
      ${openRun ? html`<${Detail} key=${openRun.id} run=${openRun} now=${now} onClose=${close} />` : null}
    </main>
  `;
}

render(html`<${App} />`, document.querySelector('#app'));
