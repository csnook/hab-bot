/** Unix seconds for a date ("2026-10-03") and time ("09:30") in local time. */
export function toUnixSeconds(date: string, time: string): number | null {
  if (!date || !time) return null;
  const ms = new Date(`${date}T${time}`).getTime();
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

export function formatTime(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString([], {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** "just now", "5 minutes ago", "2 hours ago", "3 days ago". */
export function ago(unixSeconds: number, nowSeconds: number): string {
  const s = Math.max(0, nowSeconds - unixSeconds);
  if (s < 60) return "just now";
  const unit = (n: number, name: string) => `${n} ${name}${n === 1 ? "" : "s"} ago`;
  if (s < 3600) return unit(Math.floor(s / 60), "minute");
  if (s < 86400) return unit(Math.floor(s / 3600), "hour");
  return unit(Math.floor(s / 86400), "day");
}

/** The notice other devices show when a device signs in. */
export function signInNoticeText(name: string, at: number, nowSeconds: number): string {
  return `New device signed in: ${name}, ${ago(at, nowSeconds)}.`;
}
