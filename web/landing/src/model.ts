export type Lane = "open" | "progress" | "review" | "done";

export type Person = {
  id: string;
  name: string;
  kind: "human" | "agent";
  initials: string;
};

export type Card = {
  id: string;
  title: string;
  lane: Lane;
  holder: string | null;
  stamped: boolean;
  evidence: boolean;
};

export const people: Person[] = [
  { id: "ada", name: "Ada", kind: "human", initials: "A" },
  { id: "north", name: "North", kind: "agent", initials: "N" },
  { id: "sol", name: "Sol", kind: "agent", initials: "S" },
  { id: "jun", name: "Jun", kind: "human", initials: "J" },
];

export const lanes: { id: Lane; label: string }[] = [
  { id: "open", label: "Open" },
  { id: "progress", label: "In progress" },
  { id: "review", label: "Needs review" },
  { id: "done", label: "Done" },
];

export function person(id: string | null): Person | undefined {
  return people.find((p) => p.id === id);
}

function card(
  id: string,
  title: string,
  lane: Lane,
  holder: string | null,
  stamped = false,
  evidence = false,
): Card {
  return { id, title, lane, holder, stamped, evidence };
}

/** The lane as the page opens, before anything moves. */
export function opening(): Card[] {
  return [
    card("runbook", "Draft the runbook", "open", null),
    card("keys", "Rotate session keys", "open", null),
    card("cut", "Cut the release", "progress", "sol"),
    card("migration", "Approve the migration", "review", "ada"),
    card("publish", "Publish the release", "progress", "jun"),
  ];
}

/** Where the opening motion comes to rest. */
export function settled(): Card[] {
  return opening().map((c) => {
    if (c.id === "keys") return { ...c, lane: "progress", holder: "north" };
    if (c.id === "migration") return { ...c, stamped: true };
    if (c.id === "publish") return { ...c, lane: "done", evidence: true };
    return c;
  });
}

export function holding(cards: Card[], who: string, except: string): boolean {
  return cards.some(
    (c) =>
      c.id !== except &&
      c.holder === who &&
      (c.lane === "progress" || c.lane === "review"),
  );
}

export type Step =
  | { ok: true; cards: Card[]; note: string }
  | { ok: false; cards: Card[]; note: string };

/** One tap. Open claims, review stamps, a stamped review resolves. */
export function advance(cards: Card[], id: string, who: string): Step {
  const current = cards.find((c) => c.id === id);
  if (!current) return { ok: false, cards, note: "" };
  if (current.lane === "open") {
    if (holding(cards, who, id)) {
      return {
        ok: false,
        cards,
        note: `${person(who)?.name ?? "They"} already holds a thread.`,
      };
    }
    return {
      ok: true,
      note: `${person(who)?.name ?? "Someone"} claimed ${current.title}.`,
      cards: cards.map((c) =>
        c.id === id ? { ...c, lane: "progress", holder: who } : c,
      ),
    };
  }
  if (current.lane === "progress") {
    return {
      ok: true,
      note: `${current.title} is in review.`,
      cards: cards.map((c) => (c.id === id ? { ...c, lane: "review" } : c)),
    };
  }
  if (current.lane === "review" && !current.stamped) {
    return {
      ok: true,
      note: `${current.title} was approved.`,
      cards: cards.map((c) => (c.id === id ? { ...c, stamped: true } : c)),
    };
  }
  if (current.lane === "review" && current.stamped) {
    return {
      ok: true,
      note: `${current.title} resolved. The evidence stays.`,
      cards: cards.map((c) =>
        c.id === id ? { ...c, lane: "done", evidence: true } : c,
      ),
    };
  }
  return { ok: true, cards, note: "" };
}
