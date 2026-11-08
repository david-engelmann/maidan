import { useSyncExternalStore } from "react";
import { getChoice, setChoice, subscribeTheme, type ThemeChoice } from "../core/theme.ts";

/** This browser's theme choice, and the way to change it. */
export function useTheme(): [ThemeChoice, (choice: ThemeChoice) => void] {
  const choice = useSyncExternalStore(subscribeTheme, getChoice);
  return [choice, setChoice];
}
