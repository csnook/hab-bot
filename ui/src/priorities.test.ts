import { expect, test } from "vitest";
import { formatInterval, overdueStyles } from "./priorities";
import type { PriorityInfo } from "./api";

test("intervals read the way the spec writes them", () => {
  expect(formatInterval(0)).toBe("at once");
  expect(formatInterval(86_400)).toBe("1 day");
  expect(formatInterval(3_600)).toBe("1 h");
  expect(formatInterval(600)).toBe("10 min");
  expect(formatInterval(60)).toBe("1 min");
  expect(formatInterval(45)).toBe("45 s");
});

test("escalation steps read as a sentence", () => {
  const medium = {
    settings: {
      overdue_steps: [
        { after: 0, style: "insistent" },
        { after: 3_600, style: "alarm" },
      ],
    },
  } as PriorityInfo;
  expect(overdueStyles(medium)).toBe("Insistent, then Alarm after 1 h overdue");
});
