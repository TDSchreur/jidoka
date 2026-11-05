const groupsEl = document.querySelector('#groups');
const message = document.querySelector('#message');
const GROUPS = [
  ['define', 'Define', 'Clarify the work, write a spec and create tickets.'],
  ['build', 'Build', 'Implement the spec and run the relevant checks.'],
  ['check', 'Check', 'Review the code and scan for security issues.'],
  ['finish', 'Finish', 'Create a pull request for the completed work.'],
  ['done', 'Done', 'Runs that have finished, whatever their stage.'],
];
const STATUSES = ['running', 'blocked', 'done'];
const REWORK_CAP = 2;
let lastJson = null;

const themeInputs = document.querySelectorAll('input[name="theme"]');
function applyTheme(choice) {
  if (choice === 'light' || choice === 'dark') document.documentElement.dataset.theme = choice;
  else delete document.documentElement.dataset.theme;
}
try {
  const stored = localStorage.getItem('theme');
  for (const input of themeInputs) input.checked = input.value === (stored === 'light' || stored === 'dark' ? stored : 'system');
} catch { /* storage unavailable: keep System selected */ }
for (const input of themeInputs) {
  input.addEventListener('change', () => {
    applyTheme(input.value);
    try {
      if (input.value === 'system') localStorage.removeItem('theme');
      else localStorage.setItem('theme', input.value);
    } catch { /* the choice still applies for this visit */ }
  });
}

const el = (tag, text, className) => {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
};
const timestamp = run => (typeof run.updated_at === 'number' && Number.isFinite(run.updated_at)
  && !Number.isNaN(new Date(run.updated_at).getTime())) ? run.updated_at : null;
const groupOf = run => run.status === 'done' ? 'done' : run.stage;

function compare(a, b) {
  const blocked = (b.run.status === 'blocked') - (a.run.status === 'blocked');
  if (blocked) return blocked;
  const ta = timestamp(a.run), tb = timestamp(b.run);
  if (ta !== null && tb !== null && ta !== tb) return tb - ta;
  if ((ta === null) !== (tb === null)) return ta === null ? 1 : -1;
  return b.index - a.index;
}

function ticket(value) {
  const p = el('p', undefined, 'ticket');
  p.append('Ticket: ');
  let url = null;
  try {
    const parsed = new URL(value);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') url = parsed;
  } catch { /* not a URL: show as text */ }
  if (url) {
    const a = el('a', value);
    a.href = url.href;
    a.rel = 'noopener noreferrer';
    p.append(a);
  } else {
    p.append(value);
  }
  return p;
}

function rework(count) {
  const text = count <= REWORK_CAP ? `${count} of ${REWORK_CAP}` : `${count} (past the cap of ${REWORK_CAP})`;
  return el('p', 'Rework ' + text, 'rework');
}

function card(run) {
  const item = el('li');
  const article = el('article', undefined, run.status === 'blocked' ? 'run blocked' : 'run');
  article.append(el('h3', run.title), el('p', run.project, 'project'));
  const status = STATUSES.includes(run.status) ? run.status : String(run.status);
  article.append(el('span', status, `badge ${STATUSES.includes(run.status) ? run.status : ''}`.trim()));
  if (Number.isInteger(run.rework) && run.rework > 0) article.append(rework(run.rework));
  if (run.activity) article.append(el('p', `Activity: ${run.activity}`, 'activity'));
  if (run.current_ticket) article.append(ticket(String(run.current_ticket)));
  if (run.blocking_reason) article.append(el('p', `Blocked: ${run.blocking_reason}`, 'reason'));
  const updated = el('p', undefined, 'updated');
  const ts = timestamp(run);
  if (ts !== null) {
    const date = new Date(ts);
    updated.append('Last updated ');
    const time = el('time', date.toLocaleString());
    time.dateTime = date.toISOString();
    updated.append(time);
  } else {
    updated.textContent = 'Last updated: not reported';
  }
  article.append(updated);
  item.append(article);
  return item;
}

function render(data) {
  const sections = GROUPS.map(([key, name, description]) => {
    const entries = data.map((run, index) => ({ run, index })).filter(e => groupOf(e.run) === key).sort(compare);
    const section = el('section', undefined, 'group');
    const heading = el('h2', name);
    heading.id = `group-${key}`;
    heading.append(el('span', ` (${entries.length})`, 'count'));
    section.setAttribute('aria-labelledby', heading.id);
    section.append(heading, el('p', description, 'group-description'));
    if (entries.length) {
      const list = el('ul', undefined, 'runs');
      list.append(...entries.map(e => card(e.run)));
      section.append(list);
    } else {
      section.append(el('p', 'No runs', 'empty'));
    }
    return section;
  });
  groupsEl.replaceChildren(...sections);
}

async function refresh() {
  try {
    const response = await fetch('/api/runs', { cache: 'no-store' });
    if (!response.ok) throw new Error('Failed to load runs');
    const data = await response.json();
    if (!Array.isArray(data)) throw new Error('Unexpected response');
    const json = JSON.stringify(data);
    if (json !== lastJson) {
      render(data);
      lastJson = json;
    }
    message.className = '';
    const text = data.length ? '' : 'No runs yet. Start one with the factory skill.';
    if (message.textContent !== text) message.textContent = text;
  } catch {
    message.className = 'error';
    message.textContent = 'Unable to load runs. Check that the factory server is running.';
  }
}
refresh();
setInterval(refresh, 3000);
