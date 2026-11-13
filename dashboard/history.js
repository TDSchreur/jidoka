// A Run's history as its route: the stops in order, with every report, Finding and event placed on one.
// Plain data in, plain data out: no DOM and no fetch. Stored runs may lack fields, so all checking happens here.
import { freshTokens, timestamp } from '/assets/format.js';

export const GROUPS = [
  ['define', 'Define'],
  ['build', 'Build'],
  ['check', 'Check'],
  ['finish', 'Finish'],
];
export const STATIONS = GROUPS.map(([id]) => id);
export const STATION_NAME = Object.fromEntries(GROUPS);

const FINDING_STATES = ['open', 'fixed', 'dismissed', 'deferred'];
const PAUSE_WORDS = { usage_limit: 'Usage limit reached', compaction: 'Context compaction' };

/** `value` when it is an array, else an empty one: stored runs may lack a list or hold something else. */
const listOf = (value) => (Array.isArray(value) ? value : []);

/** Appends `item` to the list stored under `key`. */
function push(map, key, item) {
  map.set(key, [...(map.get(key) ?? []), item]);
}

const reworkOf = (value) => (Number.isInteger(value) && value > 0 ? value : null);

/** Tickets with how many are done; null when there are none. */
const progressOf = (tickets) =>
  tickets.length ? { done: tickets.filter((t) => t.status === 'done').length, total: tickets.length } : null;

/** The tickets listed on a Run, with how many are done; null when none are recorded. */
export const ticketProgress = (run) => progressOf(listOf(run.tickets));

/** The run's findings that have an id, text and a known state; anything else is ignored. */
function findingsOf(run) {
  return listOf(run?.findings).filter(
    (f) => f && typeof f.id === 'string' && typeof f.text === 'string' && FINDING_STATES.includes(f.state),
  );
}

/** How many of the run's findings are in `state`. */
export const findingCount = (run, state) => findingsOf(run).filter((f) => f.state === state).length;

/** Where a station stands for a Run: done, current, blocked while it is the current one of a blocked Run, or ahead. */
function stationState(run, at) {
  if (run.status === 'done') return 'done';
  const current = STATIONS.indexOf(run.stage);
  if (at < current) return 'done';
  if (at === current) return run.status === 'blocked' ? 'blocked' : 'current';
  return 'ahead';
}

/** Fresh tokens spent in one stage, or null when the usage has no row for it. */
function stageTokens(run, id) {
  const rows = Array.isArray(run.usage?.rows) ? run.usage.rows.filter((row) => row.stage === id) : [];
  const counts = rows.map(freshTokens).filter((n) => n !== null);
  return counts.length ? counts.reduce((a, b) => a + b, 0) : null;
}

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

/**
 * The passes through the stages that the run's timeline records, oldest first: consecutive Build marks that differ
 * only by ticket form one pass that lists those tickets. `unstartedTickets` are tickets no Build pass started and
 * that have no first Build pass to wait in. `now` ends the last pass of a run that is not finished.
 */
function timelinePasses(run, now) {
  const marks = listOf(run.timeline)
    .filter((mark) => mark && Number.isFinite(mark.at) && STATIONS.includes(mark.stage))
    .sort((a, b) => a.at - b.at);
  const allTickets = listOf(run.tickets);
  const passes = [];
  for (const mark of marks) {
    const rework = reworkOf(mark.rework);
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

const shortSha = (sha) => (typeof sha === 'string' ? sha.slice(0, 7) : '');
const noun = (n) => `${n} ${n === 1 ? 'finding' : 'findings'}`;

/**
 * The Findings a pass shows, as a heading line and lists. A Check pass shows the Findings recorded in its window
 * (raised, so without outcome, when it sent the run back) and the ones dismissed or deferred in it. A Rework Build
 * pass shows the Findings it was sent to fix. `heading` is null when there is nothing to say.
 */
function passFindings(run, pass) {
  const findings = findingsOf(run);
  if (pass.stage === 'check') {
    const within = (t) => t >= pass.start && t < pass.end;
    const mine = findings.filter((f) => within(f.at));
    const resolved = findings.filter((f) => !within(f.at) && within(f.resolved_at) && (f.state === 'dismissed' || f.state === 'deferred'));
    if (!mine.length && !resolved.length && !pass.sentBack) return { heading: null, lists: [] };
    const sent = mine.filter((f) => Number.isInteger(f.rework));
    const commits = [...new Set(sent.map((f) => f.reviewed_commit).filter(Boolean))];
    const heading = pass.sentBack
      ? `Sent back${sent.length ? ` with ${noun(sent.length)}` : ''}${commits.length === 1 ? ` · reviewed ${shortSha(commits[0])}` : ''}`
      : noun(mine.length);
    const lists = [];
    if (mine.length) lists.push({ findings: mine, label: pass.sentBack ? 'Findings that sent Check back' : 'Findings recorded in this Check', outcome: !pass.sentBack });
    if (resolved.length) lists.push({ findings: resolved, label: 'Findings resolved in this Check', outcome: true });
    return { heading, lists };
  }
  if (pass.stage === 'build' && pass.rework !== null) {
    const fixing = findings.filter((f) => f.rework === pass.rework);
    if (fixing.length) return { heading: null, lists: [{ findings: fixing, label: `Findings for Rework ${pass.rework}`, outcome: true }] };
  }
  return { heading: null, lists: [] };
}

/** Pairs each ticket with its reports. */
const withReports = (tickets, reportsOf) => tickets.map((ticket) => ({ ticket, reports: reportsOf(ticket.reference) }));

function passStop(run, pass, placed) {
  return {
    kind: 'pass',
    stage: pass.stage,
    name: STATION_NAME[pass.stage],
    rework: pass.rework,
    start: pass.start,
    duration: pass.duration,
    state: pass.state,
    sentBack: pass.sentBack,
    activities: pass.activities,
    progress: progressOf(pass.tickets),
    tickets: withReports(pass.tickets, (ref) => placed.byTicket.get(ref) ?? []),
    reports: placed.reports,
    events: placed.events,
    findings: passFindings(run, pass),
    summary: (pass.lastOfStage ? run.summaries?.[pass.stage] : null) ?? null,
    tokens: null,
    cost: null,
  };
}

/** A station of a Run with no timeline: the stage's reports, and in Build the listed tickets, by stage. */
function stationStop(run, id, at) {
  const reports = placeReports(run);
  const tickets = id === 'build' ? listOf(run.tickets) : [];
  return {
    kind: 'station',
    stage: id,
    name: STATION_NAME[id],
    rework: id === 'check' ? reworkOf(run.rework) : null,
    start: null,
    duration: null,
    state: stationState(run, at),
    sentBack: false,
    activities: [],
    progress: progressOf(tickets),
    tickets: withReports(tickets, reports.byTicket),
    reports: reports.byStage(id),
    events: [],
    findings: { heading: null, lists: [] },
    summary: run.summaries?.[id] ?? null,
    tokens: stageTokens(run, id),
    cost: run.usage?.cost?.by_stage?.[id] ?? null,
  };
}

/**
 * A Run's route as plain data. `stops` are oldest first: a Pass for each pass on the run's timeline or, when it
 * has none, a Station for each stage. Every stop has the same shape. `other` holds what belongs to no stop: reports,
 * events, summaries of stages with no pass (as `[name, text]`) and tickets no pass started.
 */
export function runHistory(run, now) {
  const { passes, unstartedTickets } = timelinePasses(run, now);
  if (!passes.length) {
    return {
      stops: GROUPS.map(([id], at) => stationStop(run, id, at)),
      other: { reports: placeReports(run).other, events: validEvents(run), summaries: [], tickets: [] },
    };
  }
  const by = placeByTime(run, passes, unstartedTickets);
  return {
    stops: passes.map((pass, index) => passStop(run, pass, by.placed[index])),
    other: {
      reports: by.other,
      events: by.otherEvents,
      // A summary is kept per stage; one whose stage has no pass in the timeline (it may have been cut short) shows here.
      summaries: GROUPS.filter(([id]) => run.summaries?.[id] && !passes.some((pass) => pass.stage === id)).map(([id, name]) => [name, run.summaries[id]]),
      tickets: withReports(unstartedTickets, (ref) => by.unstarted.get(ref) ?? []),
    },
  };
}
