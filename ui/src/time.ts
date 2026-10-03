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
