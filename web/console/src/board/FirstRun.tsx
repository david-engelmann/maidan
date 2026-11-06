// The guide a brand-new workspace sees on its board: connect an agent, hand
// it the starter task, approve what it sends back. It goes away for good once
// put away, and on its own as soon as the board holds a task of the person's.
import { OAUTH_CLIENTS, TOKEN_FALLBACK, claudeOAuthCommand, mcpUrl } from "../core/connect.ts";
import { NEW_WORK_HREF, askAgentLine } from "../core/firstRun.ts";
import type { PeopleApi } from "../core/people.ts";
import { AddAgent } from "../people/AddAgent.tsx";
import { Copy } from "../tools/Connect.tsx";

export function FirstRun({ base, workspace, api, channel, channelId, onNewTask, onDismiss }: { base: string; workspace: string | null; api: PeopleApi | null; channel: string | null; channelId: string | null; onNewTask: (() => void) | null; onDismiss: () => void }) {
  const url = mcpUrl(base);
  const command = claudeOAuthCommand(base);
  return (
    <section className="card first-run" aria-labelledby="first-run-title">
      <div className="first-run-head">
        <h2 id="first-run-title">Connect your first agent</h2>
        <button type="button" className="chip-btn" onClick={onDismiss}>Dismiss</button>
      </div>
      <ol className="first-run-steps">
        <li>
          <strong>Connect an agent.</strong>
          <p className="muted">Paste this into {OAUTH_CLIENTS} and sign in when the browser opens. No token to copy.</p>
          <div className="snippet">
            <div className="snippet-head"><span className="small">MCP URL</span><Copy text={url} /></div>
            <div className="snippet-scroll"><pre className="snippet-body" tabIndex={0}>{url}</pre></div>
          </div>
          <div className="snippet">
            <div className="snippet-head"><span className="small">Claude Code, in a terminal</span><Copy text={command} /></div>
            <div className="snippet-scroll"><pre className="snippet-body" tabIndex={0}>{command}</pre></div>
          </div>
          <p className="muted small">{TOKEN_FALLBACK} Add the agent here instead: it gets its own token, never yours, and the setup for its client comes ready to paste.</p>
          {api && workspace ? (
            <AddAgent key={workspace} api={api} workspace={workspace} base={base} channelId={channelId} />
          ) : (
            <p className="muted">Sign in to add one.</p>
          )}
        </li>
        <li>
          <strong>{askAgentLine(channel)}</strong>
          <p className="muted">A “Say hello” task is waiting there. Your agent claims it, posts a hello, sets a result and sends it to review.</p>
        </li>
        <li>
          <strong>Approve it, then post your own.</strong>
          <p className="muted">It comes back in Review for you to approve. After that, post the work you want done.</p>
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
