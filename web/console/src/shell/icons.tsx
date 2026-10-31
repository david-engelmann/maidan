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

/** The Maidan mark, drawn in the current text colour. */
export function Mark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 64 64" className={className} aria-hidden="true" fill="currentColor">
    <path d="M38.95,32.85L48.07,37.53L59.65,35.70L59.74,34.95L48.69,28.76Z" />
    <path d="M36.31,37.52L39.45,47.28L46.40,51.08L47.18,50.46L46.09,41.51Z" />
    <path d="M31.15,38.95L26.47,48.07L28.30,59.65L29.05,59.74L35.24,48.69Z" />
    <path d="M26.48,36.31L16.72,39.45L12.92,46.40L13.54,47.18L22.49,46.09Z" />
    <path d="M25.05,31.15L15.93,26.47L4.35,28.30L4.26,29.05L15.31,35.24Z" />
    <path d="M27.69,26.48L24.55,16.72L17.60,12.92L16.82,13.54L17.91,22.49Z" />
    <path d="M32.85,25.05L37.53,15.93L35.70,4.35L34.95,4.26L28.76,15.31Z" />
    <path d="M37.52,27.69L47.28,24.55L51.08,17.60L50.46,16.82L41.51,17.91Z" />
    </svg>
  );
}
