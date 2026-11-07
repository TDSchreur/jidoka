import { h } from 'preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import htm from 'htm';

const html = htm.bind(h);

/** A heading and what follows it, for one topic of a tab. */
function Topic({ title, children }) {
  return html`<section><h3>${title}</h3>${children}</section>`;
}

/** Terms and their explanations. */
function Terms({ items }) {
  return html`<dl class="help-terms">
    ${items.map(([term, text]) => html`<dt key=${term}>${term}</dt><dd>${text}</dd>`)}
  </dl>`;
}

const Code = ({ children }) => html`<code>${children}</code>`;

/** The flow as a list of Stages, so a screen reader reads it in order. */
function Flow({ reworkCap }) {
  const steps = [
    ['Define', 'Clarify the work, write a spec and acceptance criteria.'],
    ['Build', 'Implement it and run the project’s checks.'],
    ['Check', `Review the code. Findings go back to Build for a Rework, up to ${reworkCap} times.`],
    ['Finish', 'Open a pull request and watch its CI.'],
    ['Shipped', 'The pull request is open and CI is green.'],
  ];
  return html`<ol class="flow">
    ${steps.map(([name, text]) => html`<li key=${name}><strong>${name}</strong><span>${text}</span></li>`)}
  </ol>`;
}

function Overview({ reworkCap }) {
  return html`
    <${Topic} title="What the factory is">
      <p>
        TDS Factory carries one piece of work, called a <strong>Run</strong>, from an idea to a pull request. You ask
        your coding agent (Codex, Claude Code or GitHub Copilot CLI) to run a workflow skill. The agent does the work in
        its own git worktree and reports each step here with the <${Code}>factory<//> command. This dashboard only shows
        that progress. It does not start or control the agent.
      </p>
    <//>
    <${Topic} title="The flow">
      <${Flow} reworkCap=${reworkCap} />
      <p>
        <${Code}>/factory-fast<//> skips Check. Every Run ends in a pull request, which only you merge. The Stages tab explains each step, and the Workflows tab explains how the three workflows
        differ.
      </p>
    <//>
    <${Topic} title="Who does what">
      <${Terms}
        items=${[
          [
            'You',
            'Describe the work, answer the questions in Define, decide when a Run shows Needs you, and review and merge the pull request.',
          ],
          [
            'Your agent',
            'Runs the workflow skill: asks questions, writes the spec, implements, runs the checks, reviews, opens the pull request and watches its CI.',
          ],
          [
            'The factory command',
            html`Records every step. The agent runs <${Code}>factory run start<//> and <${Code}>factory run update<//>; you rarely need to.`,
          ],
          [
            'The dashboard',
            'Shows every Run, grouped by project. The only change it can make is deleting the record of a Run.',
          ],
        ]}
      />
    <//>
    <${Topic} title="Getting started">
      <ol>
        <li>Install the command and the skills for your agent. The README lists the steps for each system.</li>
        <li>
          Once per repository, ask your agent to run <${Code}>/setup-matt-pocock-skills<//>. It records the issue
          tracker, the triage labels and where the domain docs live. <${Code}>/factory-full<//> needs it; the other
          workflows need it only when they read or publish specs or tickets in the tracker.
        </li>
        <li>
          Start <${Code}>factory dashboard<//> and keep it running. It listens on${' '}
          <${Code}>http://127.0.0.1:4311<//> by default.
        </li>
        <li>
          In your project, ask your agent to run <${Code}>/factory<//> to pick a workflow, or choose${' '}
          <${Code}>/factory-full<//>, <${Code}>/factory-balanced<//> or <${Code}>/factory-fast<//> yourself.
        </li>
        <li>Watch the Run here, answer when it needs you, and review the pull request when it is ready.</li>
      </ol>
    <//>
    <${Topic} title="Every Run has its own worktree">
      <p>
        A Run works in a fresh git worktree on its own branch, based on the latest remote base branch. Several Runs can
        go on at the same time, and your own checkout is never changed. The worktree stays after the pull request opens,
        so you can follow up on review comments. Remove it with <${Code}>git worktree remove<//> once the pull request
        is merged.
      </p>
    <//>
  `;
}

function Stages({ reworkCap }) {
  return html`
    <${Topic} title="Define">
      <p>
        Turn the request into something that can be built and checked: a spec, explicit acceptance criteria and, for
        bigger work, tickets. This is the Stage where the agent asks you questions.
      </p>
      <ul>
        <li>
          <strong>Full</strong> always interviews you (<${Code}>grill-with-docs<//>), records domain decisions, writes a
          spec (<${Code}>to-spec<//>) and breaks it into tickets (<${Code}>to-tickets<//>) in the issue tracker.
        </li>
        <li>
          <strong>Balanced</strong> uses a clear request as the spec and always writes acceptance criteria. It writes a
          spec once when the request needs one, makes tickets only for work with several pieces, and interviews you only
          when an ambiguity blocks the work.
        </li>
        <li>
          <strong>Fast</strong> uses a clear request with a short plan and acceptance criteria, and reaches for the
          interview, a spec or tickets only when the work needs them.
        </li>
      </ul>
      <p>
        A plan that only exists in the chat is saved in a file outside the pull request, and the Run records where, so a
        new session can continue without the original conversation.
      </p>
    <//>
    <${Topic} title="Build">
      <p>
        Implement the spec on the Run’s feature branch, in its worktree, one ticket at a time and without subagents.
        The agent runs the project’s relevant checks as it goes. Before the work leaves Build, it runs the
        project’s full verification: every command the project’s instructions say to run before merging, which
        should match its CI. A missing tool or a failing check blocks the Run until it is fixed.
      </p>
      <ul>
        <li>
          <strong>Full</strong> builds each ticket test-first with red-green cycles (<${Code}>implement-spec<//> and${' '}
          <${Code}>tdd<//>) and runs the full verification after each ticket.
        </li>
        <li>
          <strong>Balanced</strong> implements each ticket, then adds behavior tests through public interfaces
          (<${Code}>implement-spec-balanced<//>), and runs the full verification once on the finished work.
        </li>
        <li>
          <strong>Fast</strong> implements directly and writes tests only where they help
          (<${Code}>implement-spec-fast<//>), with the full verification once at the end.
        </li>
      </ul>
    <//>
    <${Topic} title="Check">
      <p>
        Full and Balanced only. The agent commits the finished work first, because the review only sees committed
        changes, and notes the reviewed commit in the Check summary. Then:
      </p>
      <ul>
        <li>
          <${Code}>code-review<//> compares everything since the Run’s base commit with two questions, answered by
          two reviewers in parallel: does the code follow the project’s standards, and does it do what the spec
          asked.
        </li>
        <li>
          <${Code}>security-review<//> looks for vulnerabilities by following the data. Full always runs it. Balanced
          runs it only when the change touches input parsing, authentication or secrets, network access, file or process
          handling, or dependencies.
        </li>
        <li>
          With no actionable findings the Run moves to Finish. Otherwise it goes back to Build for a Rework.
        </li>
        <li>
          If the Run has a model routing policy for Check, the review agents use the model and effort it names. A security review the agent runs itself stays on the
          agent’s own model. The Model routing tab explains how.
        </li>
      </ul>
    <//>
    <${Topic} title="Rework">
      <p>
        A <strong>Rework</strong> is a return from Check to Build to fix findings, followed by another Check that covers
        only the fixes. A Run gets at most ${reworkCap} Reworks; its row shows the current one as${' '}
        <em>Rework 1/${reworkCap}</em>. Findings alone never block a Run: the agent fixes them within the
        ${reworkCap} Reworks. When findings remain after the last Rework, or one cannot be resolved, the Run shows${' '}
        <em>Needs you</em> and waits for you to choose: one more Rework, finish with the findings listed in the pull
        request, or stop.
      </p>
    <//>
    <${Topic} title="Finish">
      <p>
        The agent writes the pull request description (<${Code}>pr<//>), verifies the final commit (reusing the latest
        passing verification when nothing has changed), pushes the branch and opens the pull request against the base
        branch it chose at the start. Then it watches the pull request’s CI until every check has finished. When a
        check fails it reads the log, fixes the cause in a new commit, verifies again and pushes. When another pull
        request lands first and leaves this one behind its base branch, the agent merges the base into the branch
        with a normal commit and watches CI again. Neither the CI fixes nor the base updates are Reworks. The Run has
        shipped when the pull request is open, up to date and CI is green, or when the project has no CI. If the
        failure cannot be fixed, such as an infrastructure problem or a missing permission, the Run is blocked with that
        reason.
      </p>
    <//>
    <${Topic} title="Shipped">
      <p>
        Finished Runs leave the line and go to the Shipped list below it, whatever Stage they ended in. It is a table,
        newest first, grouped by the day of the last update, with the project, the pull request and its state, the fresh tokens and
        how long ago. Select a title to open the detail. <em>Clear shipped…</em> removes the records of all shipped
        Runs after you confirm, and <em>Delete run</em> in a Run's detail removes one. Deleting removes the record from
        the dashboard only: the worktree, the branch and the pull request stay as they are. In the command line and
        the API a shipped Run has the status done.
      </p>
    <//>
    <${Topic} title="Needs you">
      <p>
        A Run that is blocked is waiting for you. The andon strip at the top turns amber and lists it with the reason,
        and its row on the line comes first, marked amber. Typical reasons:
      </p>
      <ul>
        <li>The issue tracker is not set up yet. Run <${Code}>/setup-matt-pocock-skills<//> and start again.</li>
        <li>Check findings remain after the last Rework and the agent needs your decision.</li>
        <li>A tool, a permission or a check the agent cannot fix by itself, such as a failing CI check caused by infrastructure.</li>
        <li>An answer or a decision only you can give.</li>
      </ul>
      <p>
        The <em>Needs you</em> button in the header shows only these Runs, and <em>Notify me</em> can tell you when a
        Run starts waiting.
      </p>
    <//>
  `;
}

function Workflows({ reworkCap }) {
  return html`
    <${Topic} title="Choosing a workflow">
      <p>
        Ask for <${Code}>/factory<//> and the agent assesses the request and picks one, telling you which and why. An
        explicit choice always wins. Each Run keeps its workflow, and a continued Run keeps it too.
      </p>
      <ul>
        <li>
          <strong>Full</strong> for substantial uncertainty, changes that interact, or sensitive consequences:
          authentication, authorization, secrets, destructive operations and migrations. Slowest, and uses the most
          tokens.
        </li>
        <li>
          <strong>Balanced</strong> for ordinary features and fixes, and whenever the evidence does not clearly point to
          either end.
        </li>
        <li>
          <strong>Fast</strong> for work that is clearly bounded, understood, easy to verify and low risk, such as
          documentation, configuration or an obvious fix.
        </li>
      </ul>
      <p>
        A request that only asks for analysis gets an assessment and no Run. <${Code}>/factory<//> used to run the
        quality workflow directly; that workflow is now <${Code}>/factory-full<//>, and older Runs recorded as${' '}
        <${Code}>factory<//> show as <${Code}>factory-full<//>.
      </p>
    <//>
    <${Topic} title="How they differ">
      <div class="table-scroll" role="region" aria-label="Workflow comparison" tabindex="0">
        <table class="help-table">
          <caption class="visually-hidden">The three workflows compared by Stage</caption>
          <thead>
            <tr>
              <th scope="col"></th>
              <th scope="col">Full</th>
              <th scope="col">Balanced</th>
              <th scope="col">Fast</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <th scope="row">Command</th>
              <td><${Code}>/factory-full<//></td>
              <td><${Code}>/factory-balanced<//></td>
              <td><${Code}>/factory-fast<//></td>
            </tr>
            <tr>
              <th scope="row">Define</th>
              <td>Always an interview, a spec and tickets</td>
              <td>A clear request is the spec; criteria always; tickets only for several pieces</td>
              <td>A clear request with a short plan and criteria; an interview, spec or tickets only if needed</td>
            </tr>
            <tr>
              <th scope="row">Build</th>
              <td>Test-first, with red-green cycles</td>
              <td>Implement, then add behavior tests</td>
              <td>Implement directly; tests where useful</td>
            </tr>
            <tr>
              <th scope="row">Full verification</th>
              <td>After each ticket, and after a Rework</td>
              <td>Once on the finished work, and after a Rework</td>
              <td>Once at the end</td>
            </tr>
            <tr>
              <th scope="row">Check</th>
              <td>Code review and security review, again after each Rework</td>
              <td>One code review, then a review of each Rework</td>
              <td>Skipped</td>
            </tr>
            <tr>
              <th scope="row">Security review</th>
              <td>Always</td>
              <td>Only for sensitive changes</td>
              <td>Never</td>
            </tr>
            <tr>
              <th scope="row">Reworks</th>
              <td>Up to ${reworkCap}</td>
              <td>Up to ${reworkCap}</td>
              <td>None</td>
            </tr>
            <tr>
              <th scope="row">Finish</th>
              <td colspan="3">The same for all three: pull request, then CI until green</td>
            </tr>
          </tbody>
        </table>
      </div>
    <//>
    <${Topic} title="What they share">
      <ul>
        <li>A fresh worktree and branch for every Run, and progress reported to this dashboard.</li>
        <li>Build runs one ticket at a time in one worktree, without subagents.</li>
        <li>
          The project’s own verification, taken from its agent instructions, contributing docs or build files. A
          missing tool or a failing check blocks the Run; it is never skipped.
        </li>
        <li>Prose for people (specs, tickets, summaries, commit messages, pull request text) goes through <${Code}>humanizer<//>.</li>
        <li>A pull request against the base branch, and a CI watch until it is green.</li>
      </ul>
    <//>
    <${Topic} title="Continuing a Run">
      <p>
        Start a new session and ask the agent to continue the Run, giving its id (shown in the Run detail). The agent
        reads <${Code}>factory run inspect<//>, which holds the repository, worktree, branch, base commit, spec, tickets
        and a summary per Stage, and checks them against reality. It reports every difference and never resets, cleans or
        discards anything to make them match. When everything fits, it continues from the Stage the Run was in, at the
        first ticket that is not done.
      </p>
    <//>
    <${Topic} title="Agent notes">
      <p>
        With Codex, start a new chat in Worktree mode on the intended base branch and tell the factory to use that
        worktree. Some sandboxes need permission for Git operations that write shared repository data, for network
        access, and for the dashboard. The README has the details.
      </p>
    <//>
  `;
}

/** One group of skills: a heading, and each skill with what it does and where it is used. */
function SkillGroup({ title, intro, skills }) {
  return html`<${Topic} title=${title}>
    ${intro ? html`<p>${intro}</p>` : null}
    <dl class="help-terms help-skills">
      ${skills.map(
        ([name, text, used]) => html`<dt key=${name}><${Code}>${name}<//></dt>
          <dd>${text}${used ? html`<span class="help-used">Used by: ${used}</span>` : null}</dd>`,
      )}
    </dl>
  <//>`;
}

function Skills() {
  return html`
    <p>
      Skills are instructions your agent loads when it needs them. Ask your agent to run one by name, for example${' '}
      <${Code}>/factory<//>. These are the ones the factory uses, grouped by Stage.
    </p>
    <${SkillGroup}
      title="Entry points"
      skills=${[
        [
          '/factory',
          'Assesses the request and picks a workflow, then carries on in the same session with that workflow. Does not start a Run of its own.',
          null,
        ],
        [
          '/factory-full',
          'The quality workflow: full interview, spec and tickets, test-first Build, code and security review, Reworks, pull request.',
          null,
        ],
        [
          '/factory-balanced',
          'Lighter planning than Full, behavior tests after implementing, one code review, and a security review where it matters.',
          null,
        ],
        ['/factory-fast', 'The speed workflow: light planning, direct implementation, no Check, and still a pull request with green CI.', null],
        [
          'factory-procedures',
          'The rules the three workflows share: reporting to the dashboard, continuing a Run, the worktree, tracker setup, saving the plan, model routing, writing, and the pull request and CI. The workflows read it; you do not run it yourself.',
          'all workflows',
        ],
      ]}
    />
    <${SkillGroup}
      title="Define"
      skills=${[
        [
          'grill-with-docs',
          'An interview that sharpens a plan or design one question at a time, and records the domain terms and decisions it settles in the glossary and in decision records. Builds on grilling and domain-modeling.',
          'Full always; Balanced and Fast only when unclear requirements would get in the way of the work',
        ],
        [
          'to-spec',
          'Turns what has been discussed into a spec and publishes it as an issue in the project’s tracker, labelled ready for an agent. It does not interview you.',
          'Full always; Balanced when the request needs a written spec; Fast when the task needs a lasting one',
        ],
        [
          'to-tickets',
          'Breaks a spec into small, vertical tickets that each name the tickets blocking them, and publishes them to the tracker with native blocking links where the tracker has them.',
          'Full always; Balanced and Fast for work with several dependent pieces',
        ],
      ]}
    />
    <${SkillGroup}
      title="Build"
      skills=${[
        [
          'implement-spec',
          'Implements the spec and its tickets on the feature branch, each ticket test-first with tdd, and runs the project’s verification after each ticket.',
          'Full',
        ],
        [
          'implement-spec-balanced',
          'Implements each ticket, then covers it with behavior tests through public interfaces, without red-green cycles.',
          'Balanced',
        ],
        ['implement-spec-fast', 'Implements the spec directly, with tests only where they help.', 'Fast'],
        [
          'tdd',
          'Test-driven development: write a failing test, make it pass, repeat, with rules for what a good test is and where tests belong.',
          'Full always; Balanced borrows its test-quality rules',
        ],
      ]}
    />
    <${SkillGroup}
      title="Check"
      skills=${[
        [
          'code-review',
          'Reviews the changes since a fixed commit along two axes, standards and spec, with two reviewers in parallel, and reports them side by side. Takes the model and effort a routing policy names.',
          'Full and Balanced',
        ],
        [
          'security-review',
          'Reads the code the way a security researcher would: it follows data flows, checks for exposed secrets and unsafe dependencies, double-checks its own findings and rates them by severity.',
          'Full always; Balanced for sensitive changes',
        ],
      ]}
    />
    <${SkillGroup}
      title="Finish"
      skills=${[
        [
          'pr',
          'Writes the pull request description: a summary, the evidence that it works, and how risky the merge is.',
          'all workflows',
        ],
      ]}
    />
    <${SkillGroup}
      title="Everywhere"
      skills=${[
        [
          'humanizer',
          'Rewrites prose that reads like machine text so it reads like a person, without changing facts, code or links. It edits the specs, tickets, summaries, commit messages and pull request descriptions the flow writes.',
          'all workflows',
        ],
        [
          'setup-matt-pocock-skills',
          'Run once per repository, in your normal checkout. Records which issue tracker the repository uses, its triage labels and where the domain docs live, and adds a short section to the agent instructions. Later Runs pick the output up.',
          'before the first Run',
        ],
      ]}
    />
    <p>
      Most skills come from <${Code}>mattpocock/skills<//>, <${Code}>github/awesome-copilot<//> and${' '}
      <${Code}>blader/humanizer<//>; the workflow skills are the factory’s own. <${Code}>skills/README.md<//> in the TDS Factory repository lists where each one started.
    </p>
  `;
}

function Dashboard({ reworkCap }) {
  return html`
    <${Topic} title="The board">
      <p>
        At the top, the andon strip says whether anything needs you. Below it, On the line has one row for each Run that
        is not done, with a rail of the four Stations (Define, Build, Check and Finish) that shows where the Run is. A
        blocked Run is marked amber and its position says so, such as Check, blocked. Runs waiting for you come first,
        then the most recently updated. Finished Runs are listed below the line. The board refreshes by itself every few
        seconds.
      </p>
      <p>
        Projects in the toolbar lists every project. Hide removes a project's Runs from the board in this browser, and
        Restore brings them back. Hidden projects do not count towards the waiting count or notifications, and their
        Runs are kept. Delete removes the project's Run records after you confirm; repository files, worktrees,
        branches and pull requests are kept.
      </p>
    <//>
    <${Topic} title="The header">
      <${Terms}
        items=${[
          [
            'Andon strip',
            'The bar at the top of the board. It says "Nothing needs you" and how many Runs are on the line, or turns amber and lists each Run waiting for you with its project and status reason. Select a Run to open its detail. Show only these filters the board to the waiting Runs. The number of waiting Runs also appears in the page title.',
          ],
          [
            'Notify me',
            'Asks the browser for permission to show a notification when a Run starts waiting for you, while this page is open. If the browser blocks or does not support notifications, the button says so.',
          ],
          ['Help', 'Opens this window.'],
          ['Projects', 'Lists every project, with Hide or Restore and Delete.'],
          [
            'Connection',
            'Shows whether the dashboard is receiving updates from its server, and when the last one arrived.',
          ],
        ]}
      />
      <p>
        At the bottom of the page, Theme follows your system or forces light or dark. The choice is remembered in this
        browser.
      </p>
    <//>
    <${Topic} title="A Run on the line">
      <p>Each Run on the line is a row. Select its title to open the detail.</p>
      <${Terms}
        items=${[
          ['Rail', 'The four Stations. Done ones are filled; the marker is on the current one, and turns into an amber diamond when the Run is blocked. The position is also given in words.'],
          ['Project', 'The project the Run belongs to.'],
          ['Rework', html`<em>Rework 1/${reworkCap}</em>: the current Rework out of the ${reworkCap} a Run gets.`],
          ['Tickets', 'How many of the Run’s tickets are done, such as 2/5.'],
          [
            'Tokens',
            'The fresh tokens the Run has used, such as 41.2k. It says partial when some of the usage could not be read.',
          ],
          ['Pull request, Spec', 'Links, when the Run has recorded them.'],
          ['Open, Draft, Merged, Closed', 'The state of the pull request, read from GitHub or Azure Repos by the dashboard and checked again about once a minute until it is merged. The detail says when it was checked, or why it could not be read.'],
          ['Activity', 'What the agent says it is doing.'],
          ['Updated', 'How long ago the agent last reported. It is the time of the last report, not a heartbeat.'],
        ]}
      />
    <//>
    <${Topic} title="The Run detail">
      <p>
        A panel on the right with everything the Run has recorded. <${Code}>factory run inspect<//> prints most of the same fields, and a new session reads it to continue the Run. The detail shows:
        its project, Stage, status and the reason when it is blocked, the workflow mode, branch, worktree, repository and
        base commit, links to the spec and the pull request, every ticket with its progress, a summary per Stage, the
        token usage, the Rework and the Run id. A field the agent never recorded says so. It is never guessed.
      </p>
    <//>
    <${Topic} title="Token usage">
      <p>
        The dashboard reads the token counts your agents write to their own session logs, and shows them per Stage,
        model and ticket. It counts tokens only, never money, and keeps only counts, identifiers and the reasoning effort of each call: no
        conversation text, prompts, tool output or credentials.
      </p>
      <ul>
        <li>
          <strong>Fresh tokens</strong> are the input that was not read from a cache, plus cache writes and output.
          Cache reads are shown next to them, not inside. Reasoning is part of the output.
        </li>
        <li><strong>n/a</strong> means the log did not report a number. It is never zero.</li>
        <li>
          <strong>Partial</strong> means some usage could not be attributed or read, so the totals may be too low. The
          notes say what is missing. <strong>Not tracked</strong> says why nothing is known.
        </li>
        <li>
          Subagent usage is included once, and shown as its own share. Usage is read from Claude Code, Codex and
          Copilot logs. You can turn it off with <${Code}>factory dashboard --no-usage<//> or by starting the dashboard with <${Code}>FACTORY_USAGE=off<//>.
        </li>
      </ul>
    <//>
  `;
}

function Commands() {
  return html`
    <p>
      The agent runs most of these for you. They are here for when you want to look at a Run yourself or fix a problem.
    </p>
    <${Topic} title="Look at Runs">
      <${Terms}
        items=${[
          [html`<${Code}>factory dashboard<//>`, html`Starts this dashboard on <${Code}>127.0.0.1:4311<//>. Use <${Code}>--port<//> for a separate dashboard with its own data, and <${Code}>--no-usage<//> to leave token usage off.`],
          [html`<${Code}>factory run inspect ${'<id>'}<//>`, html`Prints what a Run has recorded, the same as the Run detail. Add <${Code}>--json<//> for the stored record.`],
          [html`<${Code}>factory run stats ${'<id>'}<//>`, html`Prints the token usage by Stage, model and ticket. <${Code}>--all<//> shows the totals for every Run.`],
        ]}
      />
    <//>
    <${Topic} title="Report a Run by hand">
      <${Terms}
        items=${[
          [html`<${Code}>factory run start "title"<//>`, 'Creates a Run and prints its id. Run it from the Run’s worktree.'],
          [
            html`<${Code}>factory run update ${'<id>'} ${'<stage>'} ${'<status>'}<//>`,
            html`Stages are define, build, check and finish. Statuses are running, blocked and done. Options leave a field alone when omitted, and most have a <${Code}>--clear-${'<name>'}<//> form (tickets clear with <${Code}>--clear-tickets<//>, and a summary with <${Code}>--clear-summary<//> and a Stage name): <${Code}>--activity<//>, <${Code}>--blocking-reason<//>, <${Code}>--rework<//>, <${Code}>--mode<//>, <${Code}>--worktree<//>, <${Code}>--branch<//>, <${Code}>--base-commit<//>, <${Code}>--spec<//>, <${Code}>--pr<//>, <${Code}>--ticket<//>, <${Code}>--summary<//> and more. <${Code}>factory --help<//> lists them.`,
          ],
        ]}
      />
    <//>
    <${Topic} title="Set up and repair">
      <${Terms}
        items=${[
          [html`<${Code}>factory install --agent codex|claude|copilot<//>`, 'Installs the skills for an agent, or for several, separated by commas. It replaces the factory’s own skill folders and leaves other skills alone.'],
          [html`<${Code}>factory doctor<//>`, html`Checks whether the repository and your machine are ready, and says how to fix each problem. <${Code}>--agent<//> checks one agent, <${Code}>--json<//> prints JSON, and <${Code}>--create-model-routing<//> writes a starter model routing policy.`],
          [html`<${Code}>factory check<//>`, 'Looks up the latest release and says whether you are up to date.'],
          [html`<${Code}>factory update<//>`, 'Downloads the latest release, verifies its checksum, replaces the installed command and reinstalls the skills for the agents that already have them. Restart the dashboard afterwards.'],
          [html`<${Code}>factory --version<//>`, 'Prints the version and the build it came from.'],
        ]}
      />
    <//>
  `;
}

function ModelRouting() {
  return html`
    <${Topic} title="What it does">
      <p>
        Model routing lets you say which model and effort each kind of task should use, apart from the workflow. When a
        Run starts, the factory reads your policy once and records the choice on the Run, so editing the file later does
        not change a Run in progress. For now the role mapped to <${Code}>check<//> sets the model and effort of the
        review agents in Check. Build and Finish still run on the session’s model, and a Run without a policy runs
        as it always did.
      </p>
    <//>
    <${Topic} title="The policy file">
      <p>
        One file, <${Code}>model-routing.json<//>, in the factory’s config folder: <${Code}>$XDG_CONFIG_HOME/tds-factory/<//>
        ${' '}(or <${Code}>~/.config/tds-factory/<//>) on Linux, and <${Code}>%APPDATA%\\tds-factory\\<//> on Windows. It has an
        entry for each agent under <${Code}>agents<//> (<${Code}>claude<//> and <${Code}>codex<//>), and each entry has up to
        three roles (<${Code}>strong<//>, <${Code}>coding<//>, <${Code}>routine<//>) with a model and an effort, and says
        which role handles <${Code}>build<//>, <${Code}>check<//> and <${Code}>finish<//>. A run records only the entry of
        the agent it started in, which the workflow names with <${Code}>factory run start --agent<//>. Set${' '}
        <${Code}>"enabled": false<//> at the top, or inside an entry, to switch routing off without deleting anything.
      </p>
      <pre class="help-code"><code>{
  "enabled": true,
  "agents": {
    "claude": {
      "roles": { "strong": { "model": "opus", "effort": "high" } },
      "tasks": { "check": "strong" }
    },
    "codex": {
      "roles": { "strong": { "model": "gpt-6-astra", "effort": "high" } },
      "tasks": { "check": "strong" }
    }
  }
}</code></pre>
      <p>
        <${Code}>factory doctor --agent claude --create-model-routing<//> writes a starter like this with an entry for each agent (add${' '}
        <${Code}>--agent claude<//> or <${Code}>--agent codex<//> for one). It never overwrites a file that is already there.
      </p>
    <//>
    <${Topic} title="Checking that it works">
      <ul>
        <li>
          <${Code}>factory doctor<//> has a Model routing section. It checks that the file loads, that your agent
          supports routing, and for Claude Code that the models and efforts are ones it accepts and that nothing in the
          environment or settings overrides them. For Codex it reports what it cannot check as skipped. The section
          appears only when a policy file exists and is switched on.
        </li>
        <li>
          <${Code}>factory run inspect ${'<id>'}<//> shows, per task, the requested model and effort next to the models and
          efforts the Run’s logs show. When usage is fully tracked, a model that was never observed reads <${Code}>MISMATCH<//>, and so does an effort that differs from the one requested. With partial usage it reads "not observed", because the data may be missing, and a log that records no effort reads "effort not recorded".
        </li>
        <li>
          To give one task another role for one Run, use${' '}
          <${Code}>factory run update ${'<id>'} ${'<stage>'} ${'<status>'} --model-routing check=coding<//>.
        </li>
      </ul>
    <//>
    <${Topic} title="By agent">
      <${Terms}
        items=${[
          [
            'Claude Code',
            'The review agents start as fresh subagents with the model and effort from the policy. The model must be one of the aliases opus, sonnet, haiku or fable, and the effort option needs a recent Claude Code.',
          ],
          [
            'Codex',
            'The review agents start through Codex’s own subagent settings with the model and reasoning effort from the policy.',
          ],
          [
            'Copilot CLI',
            'Not supported. Copilot picks a subagent’s model from its own settings per agent name and offers no choice per call, so every stage there keeps the session’s model. A policy that names copilot records nothing.',
          ],
        ]}
      />
      <p>
        A coordinator and its review agents on the same model at different efforts share one usage row, so${' '}
        <${Code}>inspect<//> cannot tell their efforts apart. When the policy cannot be applied, the Check summary says
        why.
      </p>
    <//>
  `;
}

const TABS = [
  ['overview', 'Overview', Overview],
  ['stages', 'Stages', Stages],
  ['workflows', 'Workflows', Workflows],
  ['skills', 'Skills', Skills],
  ['dashboard', 'Dashboard', Dashboard],
  ['commands', 'Commands', Commands],
  ['routing', 'Model routing', ModelRouting],
];

/**
 * The dashboard's help: what the factory does, Stage by Stage, which workflows and skills it uses, how to read
 * the board and which commands exist. A native modal, so focus moves into it and back to what opened it on close.
 * The tabs follow the WAI-ARIA tabs pattern: arrow keys, Home and End move and select, and only the selected tab
 * is in the tab order. Each tab's content scrolls inside the window.
 */
export function Help({ onClose, reworkCap }) {
  const dialog = useRef(null);
  const closing = useRef(onClose);
  closing.current = onClose;
  const tabs = useRef([]);
  const [current, setCurrent] = useState(TABS[0][0]);
  useEffect(() => {
    const element = dialog.current;
    if (!element.open) element.showModal();
    const onNativeClose = () => closing.current();
    element.addEventListener('close', onNativeClose);
    return () => element.removeEventListener('close', onNativeClose);
  }, []);
  const choose = (index) => {
    setCurrent(TABS[index][0]);
    tabs.current[index]?.focus();
  };
  const onKeyDown = (event, index) => {
    const last = TABS.length - 1;
    let target;
    switch (event.key) {
      case 'ArrowRight':
        target = index === last ? 0 : index + 1;
        break;
      case 'ArrowLeft':
        target = index === 0 ? last : index - 1;
        break;
      case 'Home':
        target = 0;
        break;
      case 'End':
        target = last;
        break;
      default:
        return;
    }
    event.preventDefault();
    choose(target);
  };
  const [, , Content] = TABS.find(([id]) => id === current);
  return html`<dialog ref=${dialog} class="confirm help" aria-labelledby="help-title">
    <div class="help-bar">
      <h2 id="help-title">Help</h2>
      <button type="button" class="toggle help-close" onClick=${() => dialog.current.close()}>Close</button>
    </div>
    <div class="help-tabs" role="tablist" aria-label="Help topics">
      ${TABS.map(
        ([id, name], index) => html`<button
          key=${id}
          ref=${(element) => (tabs.current[index] = element)}
          type="button"
          role="tab"
          id=${`help-tab-${id}`}
          class="help-tab"
          aria-selected=${String(id === current)}
          aria-controls="help-panel"
          tabindex=${id === current ? 0 : -1}
          autofocus=${id === current}
          onClick=${() => choose(index)}
          onKeyDown=${(event) => onKeyDown(event, index)}
        >
          ${name}
        </button>`,
      )}
    </div>
    <div key=${current} id="help-panel" class="help-panel" role="tabpanel" aria-labelledby=${`help-tab-${current}`} tabindex="0">
      <${Content} reworkCap=${reworkCap} />
    </div>
  </dialog>`;
}
