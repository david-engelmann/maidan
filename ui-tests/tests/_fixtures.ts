import { readFileSync } from "fs";
import { resolve } from "path";

/** The deterministic seed written by the ui_test_server harness. */
export interface Fixtures {
  base_url: string;
  token: string;
  workspace_id: string;
  member_id: string;
  channel_id: string;
  thread_id: string;
  gate_id: string;
}

export function fixtures(): Fixtures {
  return JSON.parse(readFileSync(resolve(__dirname, "../.fixtures.json"), "utf-8"));
}
