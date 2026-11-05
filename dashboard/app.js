import { h, render } from 'preact';
import { useEffect, useState } from 'preact/hooks';
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

function Ticket({ value }) {
  let url = null;
  try {
    const parsed = new URL(value);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') url = parsed;
  } catch {
    /* not a URL: show as text */
  }
  return html`<p class="ticket">
    Ticket: ${url ? html`<a href=${url.href} rel="noopener noreferrer">${value}</a>` : value}
  </p>`;
}

function Updated({ run }) {
  const ts = timestamp(run);
  if (ts === null) return html`<p class="updated">Last updated: not reported</p>`;
  const date = new Date(ts);
  return html`<p class="updated">
    Last updated${' '}<time datetime=${date.toISOString()}>${date.toLocaleString()}</time>
  </p>`;
}

function Card({ run }) {
  const known = STATUSES.includes(run.status);
  const rework = Number.isInteger(run.rework) && run.rework > 0 ? run.rework : null;
  return html`<li>
    <article class=${run.status === 'blocked' ? 'run blocked' : 'run'}>
      <h3>${run.title}</h3>
      <p class="project">${run.project}</p>
      <span class=${known ? `badge ${run.status}` : 'badge'}>${String(run.status)}</span>
      ${rework !== null
        ? html`<p class="rework">
            Rework${' '}${rework <= REWORK_CAP
              ? `${rework} of ${REWORK_CAP}`
              : `${rework} (past the cap of ${REWORK_CAP})`}
          </p>`
        : null}
      ${run.activity ? html`<p class="activity">Activity: ${run.activity}</p>` : null}
      ${run.current_ticket ? html`<${Ticket} value=${String(run.current_ticket)} />` : null}
      ${run.blocking_reason ? html`<p class="reason">Blocked: ${run.blocking_reason}</p>` : null}
      <${Updated} run=${run} />
    </article>
  </li>`;
}

function Group({ id, name, description, entries }) {
  return html`<section class="group" aria-labelledby=${`group-${id}`}>
    <div class="group-header">
      <h2 id=${`group-${id}`}>${name}<span class="count">${` (${entries.length})`}</span></h2>
      <p class="group-description">${description}</p>
    </div>
    ${entries.length
      ? html`<ul class="runs">
          ${entries.map((entry) => html`<${Card} key=${entry.index} run=${entry.run} />`)}
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

function App() {
  const { runs, failed } = useRuns();
  let text = 'Loading runs…';
  if (failed) text = 'Unable to load runs. Check that the factory server is running.';
  else if (runs) text = runs.length ? '' : 'No runs yet. Start one with the factory skill.';
  const indexed = (runs ?? []).map((run, index) => ({ run, index }));
  return html`
    <${ThemeSwitch} />
    <main>
      <p id="message" role="status" class=${failed ? 'error' : ''}>${text}</p>
      <div id="groups">
        ${runs
          ? GROUPS.map(
              ([id, name, description]) => html`<${Group}
                key=${id}
                id=${id}
                name=${name}
                description=${description}
                entries=${indexed.filter((entry) => groupOf(entry.run) === id).sort(compare)}
              />`,
            )
          : null}
      </div>
    </main>
  `;
}

render(html`<${App} />`, document.querySelector('#app'));
