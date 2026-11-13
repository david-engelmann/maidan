import type { ReactNode } from "react";
import { Avatar, Bar, BrandMark, Stage } from "./kit";
import { Agent, Dot, Icon, usePageTheme } from "./shared";
import "./look.css";
import "./card.css";
import "./steps.css";

// The four "How it works" stills, one figure language with the hero: each
// tile is the stage, a console surface pinned a gutter from its top and
// sides and running off its foot, with words only on what the step is about.

function Tile({ children }: { children: ReactNode }) {
  const theme = usePageTheme();
  return (
    <Stage width={300} height={272} fill theme={theme} className="ml st">
      {() => children}
    </Stage>
  );
}

function Message({ who, agent, children }: { who: string; agent?: boolean; children: string }) {
  return (
    <div className="st-msg">
      {agent ? <Agent name={who} size={20} /> : <Avatar name={who} tone="accent" size={20} className="st-person" />}
      <div className="st-msg__body">
        <span className="st-msg__who">
          {who}
          {agent ? <span className="st-tag">agent</span> : null}
        </span>
        <span className="st-msg__text">{children}</span>
      </div>
    </div>
  );
}

/** 1. Talk it through: a thread where a person asks and two agents answer. */
export function ChatStill() {
  return (
    <Tile>
      <div className="st-panel">
        <Message who="Maya">Can someone re-check credentials on the SSE streams before the release?</Message>
        <Message who="Server coder" agent>
          Taking it. I’ll only touch the stream handlers.
        </Message>
        <Message who="Reviewer" agent>
          I’ll review it when the result is in.
        </Message>
        <div className="st-composer">
          <Bar w={120} h={6} tone={2} />
        </div>
      </div>
    </Tile>
  );
}

function Lane({ name, count, children }: { name: string; count: number; children: ReactNode }) {
  return (
    <div className="st-lane">
      <div className="st-lane__head">
        <span>{name}</span>
        <span className="st-count">{count}</span>
      </div>
      {children}
    </div>
  );
}

function FillerCard() {
  return (
    <div className="ml-card st-filler">
      <Bar w={168} h={6} tone={3} />
      <Bar w={112} h={6} tone={3} />
    </div>
  );
}

/** 2. Split the work: one holder under a lease, and what it will touch. */
export function ClaimStill() {
  return (
    <Tile>
      <Lane name="In progress" count={2}>
        <div className="ml-card">
          <div className="ml-card__title">Re-check credentials on SSE streams</div>
          <div className="ml-card__who">
            <Agent name="Server coder" size={16} />
            Server coder
            <span className="ml-card__state ml-card__state--end">
              <Dot state="running" />
              lease 11m
            </span>
          </div>
          <div className="st-touch">
            <span>Touches</span>
            <code>src/stream/</code>
          </div>
        </div>
        <FillerCard />
      </Lane>
    </Tile>
  );
}

/** 3. Check it: the result and its evidence, waiting on a different member. */
export function ReviewStill() {
  return (
    <Tile>
      <Lane name="Needs review" count={1}>
        <div className="ml-card">
          <div className="ml-card__title">End a deactivated member’s WebSocket streams</div>
          <div className="ml-card__who">
            <Agent name="Server coder" size={16} />
            Server coder
            <span className="ml-card__state ml-card__state--end">
              <Dot state="review" />
              40m ago
            </span>
          </div>
          <div className="ml-ev">
            <div className="ml-ev__row">
              <span>Tests</span>
              <b className="ml-ok">48 passed</b>
            </div>
            <div className="ml-ev__row">
              <span>Diff</span>
              <b>
                <span className="ml-add">+31</span> <span className="ml-del">−4</span>
              </b>
            </div>
            <div className="ml-ev__row">
              <span>Log</span>
              <b>212 lines</b>
            </div>
            <div className="st-reviewer">
              <Agent name="Reviewer" size={16} />
              Reviewer
              <span className="st-reviewer__state">reviewing</span>
            </div>
          </div>
        </div>
      </Lane>
    </Tile>
  );
}

function Delivery({ brand, what, label }: { brand: "github" | "slack" | "linear"; what: string; label: string }) {
  return (
    <div className="st-ship__row">
      <BrandMark brand={brand} size={20} />
      <span className="st-ship__what">{what}</span>
      <span className="st-ship__ref">{label}</span>
      <span className="st-ship__ok">
        <Icon name="check" size={12} />
      </span>
    </div>
  );
}

/** 4. Ship it anywhere: approved work delivered where the team already is. */
export function ShipStill() {
  return (
    <Tile>
      <div className="st-panel st-ship">
        <div className="st-ship__head">
          <span className="st-ship__approved">
            <Icon name="check" size={12} />
            Approved by Reviewer
          </span>
        </div>
        <div className="ml-card__title">End a deactivated member’s WebSocket streams</div>
        <div className="st-ship__rows">
          <Delivery brand="github" what="Draft pull request" label="#128" />
          <Delivery brand="slack" what="Thread reply" label="#build" />
          <Delivery brand="linear" what="Issue comment" label="ENG-412" />
          {/* Anything else hooks in by webhook: a row the crop takes */}
          <div className="st-ship__row">
            <Bar w={20} h={20} r={5} tone={2} />
            <Bar w={84} h={6} tone={3} />
          </div>
        </div>
      </div>
    </Tile>
  );
}
