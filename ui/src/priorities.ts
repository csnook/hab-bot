import type { AlertStyle, PriorityInfo } from "./api";

/** "1 day", "1 h", "10 min", "60 s"; 0 is "at once". */
export function formatInterval(seconds: number): string {
  if (seconds === 0) return "at once";
  if (seconds % 86_400 === 0) {
    const d = seconds / 86_400;
    return d === 1 ? "1 day" : `${d} days`;
  }
  if (seconds % 3_600 === 0) return `${seconds / 3_600} h`;
  if (seconds % 60 === 0) return `${seconds / 60} min`;
  return `${seconds} s`;
}

const STYLES: Record<AlertStyle, string> = {
  silent: "Silent",
  gentle: "Gentle",
  insistent: "Insistent",
  alarm: "Alarm",
};

/** "Insistent, then Alarm after 1 h overdue". */
export function overdueStyles(p: PriorityInfo): string {
  const steps = p.settings.overdue_steps;
  return steps
    .map((s, i) =>
      i === 0 ? STYLES[s.style] : `then ${STYLES[s.style]} after ${formatInterval(s.after)} overdue`,
    )
    .join(", ");
}

export const styleName = (s: AlertStyle) => STYLES[s];
