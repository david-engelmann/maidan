import type { RouteId } from "./router.ts";

const PATHS: Record<RouteId | "search", string> = {
  board: "M4 5h4v14H4zM10 5h4v9h-4zM16 5h4v6h-4z",
  needs: "M12 3a6 6 0 0 0-6 6v4l-2 3h16l-2-3V9a6 6 0 0 0-6-6zM10 19a2 2 0 0 0 4 0",
  threads: "M4 5h16v10H9l-5 4z",
  people: "M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM3 19c0-3 3-5 6-5s6 2 6 5M17 11a2.5 2.5 0 1 0 0-5M17 14c2 0 4 1.5 4 4",
  tools: "M14 6a4 4 0 0 0-5 5l-5 5 3 3 5-5a4 4 0 0 0 5-5l-2 2-2-1-1-2z",
  artifacts: "M7 3h7l5 5v13H7zM14 3v5h5",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
};

export function Icon({ name }: { name: RouteId | "search" }) {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}
