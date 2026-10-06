/**
 * Deleting a reminder asks whether to keep its history, marked deleted, or to
 * delete it with its history (spec: Desktop → Notices).
 */
export type DeleteChoice = "keep" | "purge";

export const DELETE_CHOICES: Array<{ value: DeleteChoice; label: string; detail: string }> = [
  {
    value: "keep",
    label: "Keep its history, marked deleted",
    detail: "It stops firing and alerting on all your devices. What happened to it stays on record.",
  },
  {
    value: "purge",
    label: "Delete it with its history",
    detail:
      "Everything about it is removed from every device that syncs. Copies already on a device that hasn't synced, or that was removed from your account, can't be recalled.",
  },
];

/** The title of the confirmation. */
export const deleteQuestion = (title: string): string => `Delete “${title}”?`;

/** What the core is asked for. */
export const withHistory = (choice: DeleteChoice): boolean => choice === "purge";
