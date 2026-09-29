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
  /** The `desk` channel: three tasks in review naming the operator as reviewer. */
  desk_channel_id: string;
  desk_approve_thread_id: string;
  desk_send_back_thread_id: string;
  /** Stays in review: a token without thread:transition cannot approve it. */
  desk_waiting_thread_id: string;
  /** The `floor` channel: the deployer holds one task; one is open to claim. */
  floor_channel_id: string;
  floor_held_thread_id: string;
  floor_glide_thread_id: string;
  floor_jump_thread_id: string;
}

export function fixtures(): Fixtures {
  return JSON.parse(readFileSync(resolve(__dirname, "../.fixtures.json"), "utf-8"));
}
