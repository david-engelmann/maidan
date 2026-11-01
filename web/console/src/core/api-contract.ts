// Compile-time contract between the console's read shapes and the server's
// OpenAPI schemas (api.gen.ts, generated from the committed spec). Each line
// says "a body the server documents fits what the console reads". When a
// field the console relies on is renamed, retyped or dropped in the spec,
// `npm run typecheck` (and `npm run check:spec`) fails on that line.
import type * as Api from "./api.gen";
import type { Message, GroupDm, Edit, VoteRow } from "./threads";
import type { Thread, Channel, Transition } from "./board";
import type { Notification, NotificationPref, Depth, SlashCommand, Schedule, Peer, AuditEvent } from "./tools";

type Fits<Server, Console> = [Server] extends [Console] ? true : false;
const fits = <T extends true>(): T | undefined => undefined;

export const contract = [
  // The spec leaves `metadata` untyped ({}); the console reads it as an object bag.
  fits<Fits<Omit<Api.Message, "metadata">, Omit<Message, "metadata">>>(),
  fits<Fits<Api.GroupDmConversation, GroupDm>>(),
  fits<Fits<Api.MessageEdit, Edit>>(),
  fits<Fits<Api.Vote, VoteRow>>(),
  fits<Fits<Api.Thread, Thread>>(),
  fits<Fits<Api.Channel, Channel>>(),
  fits<Fits<Api.ThreadTransition, Transition>>(),
  fits<Fits<Api.Notification, Notification>>(),
  fits<Fits<Api.NotificationPref, NotificationPref>>(),
  fits<Fits<Api.QueueDepth, Depth>>(),
  fits<Fits<Api.SlashCommandResponse, SlashCommand>>(),
  fits<Fits<Api.TaskSchedule, Schedule>>(),
  fits<Fits<Api.PeerResponse, Peer>>(),
  fits<Fits<Api.AuditEvent, AuditEvent>>(),
];
