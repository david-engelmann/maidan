import { readFileSync } from "fs";
import { resolve } from "path";

/** The deterministic seed written by the ui_test_server harness. */
export interface Fixtures {
  base_url: string;
  token: string;
  /** A second member's token, for opening a gate the operator then answers. */
  requester_token: string;
  /** The operator with workspace:read + event:subscribe, for the Live bar. */
  live_token: string;
  workspace_id: string;
  member_id: string;
  channel_id: string;
  thread_id: string;
  gate_id: string;
  /** The requesting agent ("Deployer"), who holds the board's claimed thread. */
  requester_id: string;
  /** The `build` channel: one thread per board lane. */
  board_channel_id: string;
  board_open_thread_id: string;
  board_claimed_thread_id: string;
  board_review_thread_id: string;
  board_done_thread_id: string;
  /** The operator with thread:transition, to approve and close. */
  review_token: string;
  /** The operator with token:admin, to mint throwaway tokens a spec may rotate. */
  admin_token: string;
  /** The `desk` channel: three tasks in review naming the operator as reviewer. */
  desk_channel_id: string;
  desk_approve_thread_id: string;
  desk_send_back_thread_id: string;
  /** Stays in review: a token without thread:transition cannot approve it. */
  desk_waiting_thread_id: string;
  /** The `triage` channel: two reviews that name no reviewer. */
  triage_channel_id: string;
  /** Owned by the operator, with a result and no review requirement. */
  triage_owned_thread_id: string;
  /** Owned by nobody and with no result: it falls to the workspace admins. */
  triage_ownerless_thread_id: string;
  /** The `hold` channel: a task the operator owns, unblocked until a spec blocks it. */
  hold_channel_id: string;
  hold_thread_id: string;
  /** A second workspace, for showing one workspace sees nothing of another's blocks. */
  second_workspace_id: string;
  second_member_id: string;
  /** The stranger's grant in the second workspace (every capability). */
  second_token: string;
  /** Owned by the stranger and blocked (reason human) from the seed. */
  second_thread_id: string;
  /** The `floor` channel: the deployer holds one task; one is open to claim. */
  floor_channel_id: string;
  floor_held_thread_id: string;
  floor_glide_thread_id: string;
  floor_jump_thread_id: string;
  /** The `quiet` channel: no tasks, for the onboarding state. */
  quiet_channel_id: string;
  /** The `lab` channel: markup and script URLs in every field, for the injection audit. */
  lab_channel_id: string;
  lab_thread_id: string;
  /** The lab member. A third person, so a group DM can be opened. */
  lab_member_id: string;
  /** Rae Reviewer, a second human, so the member picker offers a human. */
  rae_member_id: string;
  /** A second workspace: its Visitor (human), Outsider (agent) and the Visitor's token. */
  other_workspace_id: string;
  other_member_id: string;
  outsider_member_id: string;
  other_token: string;
  /** Admin grant (token:admin and the worker preset). Connect an agent uses it. */
  admin_token: string;
  /** A dead-lettered webhook delivery the Operator tab can replay. */
  delivery_id: number;
  delivery_url: string;
}

export function fixtures(): Fixtures {
  return JSON.parse(readFileSync(resolve(__dirname, "../.fixtures.json"), "utf-8"));
}
