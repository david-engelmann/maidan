// The guide a brand-new workspace sees on its empty board: connect an agent,
// give it its own token, post the first task. It goes away for good once put
// away, and on its own as soon as the board holds a task.
import { claudeOAuthCommand } from "../core/connect.ts";
import { NEW_WORK_HREF } from "../core/firstRun.ts";
import { Copy } from "../tools/Connect.tsx";

export function FirstRun({ base, channel, onNewTask, onDismiss }: { base: string; channel: string | null; onNewTask: (() => void) | null; onDismiss: () => void }) {
  const command = claudeOAuthCommand(base);
  return (
    <section className="card first-run" aria-labelledby="first-run-title">
      <div className="first-run-head">
        <h2 id="first-run-title">Connect your first agent</h2>
        <button type="button" className="chip-btn" onClick={onDismiss}>Dismiss</button>
      </div>
      <ol className="first-run-steps">
        <li>
          <strong>Add Maidan to Claude Code.</strong>
          <p className="muted">It needs no token: the first time Claude Code connects, your browser opens so you can sign in.</p>
          <div className="snippet">
            <div className="snippet-head"><span className="small">Terminal</span><Copy text={command} /></div>
            <div className="snippet-scroll"><pre className="snippet-body">{command}</pre></div>
          </div>
        </li>
        <li>
          <strong>Using another client, or your own loop?</strong>
          <p className="muted">
            Mint an admin token once in <a href="#/tools/connect">Tools, then Connect</a>. Sign out and sign back in here with that token, then mint a token for each agent in <a href="#/people">People</a>, with only what that agent needs. Never give the admin token to an agent.
          </p>
        </li>
        <li>
          <strong>{channel ? `Post a task in #${channel}.` : "Post your first task."}</strong>
          <p className="muted">Your agent claims it, does the work, and hands it back for review.</p>
          {channel && onNewTask ? (
            <button type="button" className="button ink" onClick={onNewTask}>Post a task</button>
          ) : (
            <a className="button ink" href={NEW_WORK_HREF}>Create a channel</a>
          )}
        </li>
      </ol>
    </section>
  );
}
