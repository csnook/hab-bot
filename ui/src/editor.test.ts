import { describe, expect, test } from "vitest";
import type { ReminderView } from "./api";
import {
  buildEdit,
  buildNew,
  buildPause,
  expirySpecs,
  newExpiry,
  newNextExpiry,
  listMove,
  newReminderList,
  newState,
  overdueSpec,
  stateFromView,
  summaryInput,
  type EditorState,
} from "./editor";
import { summarize } from "./summary";

const NOW = 1_790_000_000;
const ZONE = "Europe/London";

const filled = (patch: Partial<EditorState> = {}): EditorState => ({
  ...newState(),
  title: "Take the bins out",
  date: "2026-10-07",
  time: "18:00",
  ...patch,
});

const view = (patch: Partial<ReminderView> = {}): ReminderView => ({
  reminder_id: "r1",
  list_id: "l1",
  list_name: null,
  list_colour: null,
  title: "Take the bins out",
  note: "",
  priority: "medium",
  trigger: {
    kind: "schedules",
    schedules: [
      {
        parts: { pattern: { kind: "weekly", days: ["WE"] }, date: "2026-10-07", time: "18:00" },
        start: "2026-10-07T18:00:00",
        rule: "FREQ=WEEKLY;BYDAY=WE",
      },
    ],
  },
  zone: null,
  default_overdue_seconds: 3600,
  overdue: null,
  pause: null,
  list_pause: null,
  expiries: [],
  ...patch,
});

describe("creating", () => {
  test("a plain one-off sends no extras", () => {
    const r = buildNew(filled(), NOW, ZONE);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value.plan.kind).toBe("once");
    expect(r.value.priority).toBe("medium");
    expect(r.value.extras).toBeNull();
  });

  test("the note, an overdue override and expiries go in the extras", () => {
    const r = buildNew(
      filled({
        note: "  Green bin too ",
        overdue: { mode: "after", amount: 3, unit: "hours" },
        expiries: [newExpiry(), newNextExpiry()],
      }),
      NOW,
      ZONE,
    );
    expect(r.ok && r.value.extras).toEqual({
      note: "Green bin too",
      overdue: { kind: "after", seconds: 10800 },
      expiry: [
        { kind: "after", seconds: 3600 },
        { kind: "next", pattern: { kind: "daily" }, time: "23:59" },
      ],
    });
  });

  test("a repeating reminder pins to the zone only when asked", () => {
    const loose = buildNew(filled({ repeat: "daily" }), NOW, ZONE);
    const pinned = buildNew(filled({ repeat: "daily", pinned: true }), NOW, ZONE);
    expect(loose.ok && loose.value.plan).toMatchObject({ kind: "schedule", zone: null });
    expect(pinned.ok && pinned.value.plan).toMatchObject({ kind: "schedule", zone: ZONE });
  });

  test("a countdown", () => {
    const r = buildNew(
      filled({ repeat: "countdown", amount: 3, unit: "days", timeOfDay: "09:00", pinned: true }),
      NOW,
      ZONE,
    );
    expect(r.ok && r.value.plan).toEqual({
      kind: "countdown",
      title: "Take the bins out",
      countdown: { amount: 3, unit: "days", at: "09:00" },
      lastDone: NOW,
      zone: ZONE,
    });
  });

  test("the first mistake is named", () => {
    const err = (s: EditorState) => {
      const r = buildNew(s, NOW, ZONE);
      return r.ok ? null : r.error;
    };
    expect(err(filled({ title: "  " }))).toMatch(/title/);
    expect(err(filled({ date: "" }))).toMatch(/date and time/);
    expect(err(filled({ repeat: "weekly", days: [] }))).toMatch(/days/);
    expect(err(filled({ overdue: { mode: "after", amount: 0, unit: "hours" } }))).toMatch(
      /whole number/,
    );
    expect(err(filled({ expiries: [{ kind: "after", amount: 1.5, unit: "hours" }] }))).toMatch(
      /whole number/,
    );
    expect(
      err(
        filled({
          expiries: [
            { kind: "next", next: { repeat: "weekly", days: [], day: 1, ordinal: 1, weekday: "MO", time: "09:00" } },
          ],
        }),
      ),
    ).toMatch(/day/);
    expect(err(filled({ repeat: "countdown", amount: 0 }))).toMatch(/whole number/);
  });
});

describe("reading an existing reminder into the form", () => {
  test("a schedule", () => {
    const s = stateFromView(view());
    expect(s).toMatchObject({
      repeat: "weekly",
      days: ["WE"],
      date: "2026-10-07",
      time: "18:00",
      pinned: false,
      whenLocked: null,
      whenTouched: false,
    });
  });

  test("a pinned zone, a note, an override and expiries come back", () => {
    const s = stateFromView(
      view({
        zone: "Asia/Tokyo",
        note: "Green bin too",
        overdue: { kind: "next", pattern: { kind: "monthly_by_date", day: 1 }, time: "00:00" },
        expiries: [
          { kind: "after", seconds: 7200 },
          { kind: "next", pattern: { kind: "daily" }, time: "23:59" },
        ],
      }),
    );
    expect(s.pinned).toBe(true);
    expect(s.note).toBe("Green bin too");
    expect(s.overdue).toMatchObject({ mode: "next", next: { repeat: "monthly_date", day: 1 } });
    expect(s.expiries).toMatchObject([
      { kind: "after", amount: 2, unit: "hours" },
      { kind: "next", next: { repeat: "daily", time: "23:59" } },
    ]);
  });

  test("a hand-written schedule locks the When and is kept as it is", () => {
    const v = view({
      trigger: {
        kind: "schedules",
        schedules: [{ parts: null, start: "2026-10-07T18:00:00", rule: "FREQ=YEARLY" }],
      },
      overdue: { kind: "other", rule: "FREQ=YEARLY" },
      expiries: [{ kind: "other", rule: "FREQ=YEARLY" }],
    });
    const s = stateFromView(v);
    expect(s.whenLocked).toMatch(/FREQ=YEARLY/);
    // Saving without touching anything changes nothing, even though the specs
    // can't be built from the form.
    const r = buildEdit({ ...s, whenTouched: true }, v, ZONE);
    expect(r).toEqual({ ok: true, value: {} });
  });

  test("several schedules lock the When too", () => {
    const sch = view().trigger;
    if (sch.kind !== "schedules") throw new Error();
    const s = stateFromView(view({ trigger: { kind: "schedules", schedules: [...sch.schedules, ...sch.schedules] } }));
    expect(s.whenLocked).toMatch(/2 schedules/);
  });

  test("a countdown and a one-off", () => {
    const c = stateFromView(
      view({ trigger: { kind: "countdown", countdown: { amount: 2, unit: "weeks", at: "08:00" } } }),
    );
    expect(c).toMatchObject({ repeat: "countdown", amount: 2, unit: "weeks", timeOfDay: "08:00" });
    const o = stateFromView(view({ trigger: { kind: "one_off", fire_at: NOW } }));
    expect(o.repeat).toBe("once");
    expect(o.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(o.time).toMatch(/^\d{2}:\d{2}$/);
  });
});

describe("saving an edit sends only what changed", () => {
  const v = view();
  const same = () => stateFromView(v);

  test("nothing changed, nothing sent", () => {
    expect(buildEdit(same(), v, ZONE)).toEqual({ ok: true, value: {} });
    expect(buildEdit({ ...same(), whenTouched: true }, v, ZONE)).toMatchObject({ ok: true });
  });

  test("title, note and priority", () => {
    const r = buildEdit({ ...same(), title: " Bins ", note: "x", priority: "high" }, v, ZONE);
    expect(r).toEqual({ ok: true, value: { title: "Bins", note: "x", priority: "high" } });
  });

  test("overriding overdue, then going back to the priority", () => {
    const over = buildEdit({ ...same(), overdue: { mode: "after", amount: 2, unit: "hours" } }, v, ZONE);
    expect(over).toEqual({ ok: true, value: { overdue: { kind: "after", seconds: 7200 } } });
    const had = view({ overdue: { kind: "after", seconds: 7200 } });
    const back = buildEdit({ ...stateFromView(had), overdue: { mode: "default" } }, had, ZONE);
    expect(back).toEqual({ ok: true, value: { follow_priority: true } });
    const keep = buildEdit(stateFromView(had), had, ZONE);
    expect(keep).toEqual({ ok: true, value: {} });
  });

  test("expiries are sent whole, so removing the last one clears them", () => {
    const withTwo = view({ expiries: [{ kind: "after", seconds: 3600 }] });
    const add = buildEdit(
      { ...stateFromView(withTwo), expiries: [...stateFromView(withTwo).expiries, newNextExpiry()] },
      withTwo,
      ZONE,
    );
    expect(add).toEqual({
      ok: true,
      value: {
        expiry: [
          { kind: "after", seconds: 3600 },
          { kind: "next", pattern: { kind: "daily" }, time: "23:59" },
        ],
      },
    });
    const clear = buildEdit({ ...stateFromView(withTwo), expiries: [] }, withTwo, ZONE);
    expect(clear).toEqual({ ok: true, value: { expiry: [] } });
  });

  test("the When is sent only once touched", () => {
    const moved = { ...same(), time: "19:00" };
    expect(buildEdit(moved, v, ZONE)).toEqual({ ok: true, value: {} });
    expect(buildEdit({ ...moved, whenTouched: true }, v, ZONE)).toEqual({
      ok: true,
      value: { schedule: { pattern: { kind: "weekly", days: ["WE"] }, date: "2026-10-07", time: "19:00" } },
    });
    const bad = buildEdit({ ...same(), whenTouched: true, days: [] }, v, ZONE);
    expect(bad.ok).toBe(false);
  });

  test("a one-off's time and a countdown's settings", () => {
    const one = view({ trigger: { kind: "one_off", fire_at: NOW } });
    const s = { ...stateFromView(one), whenTouched: true, date: "2030-01-01", time: "10:00" };
    const r = buildEdit(s, one, ZONE);
    expect(r.ok && r.value.fire_at).toBe(new Date("2030-01-01T10:00").getTime() / 1000);
    const cd = view({ trigger: { kind: "countdown", countdown: { amount: 2, unit: "weeks", at: null } } });
    const c = buildEdit({ ...stateFromView(cd), whenTouched: true, amount: 4 }, cd, ZONE);
    expect(c).toEqual({ ok: true, value: { countdown: { amount: 4, unit: "weeks", at: null } } });
  });

  test("pinning and unpinning the time zone", () => {
    expect(buildEdit({ ...same(), pinned: true }, v, ZONE)).toEqual({ ok: true, value: { zone: ZONE } });
    const pinned = view({ zone: "Asia/Tokyo" });
    expect(buildEdit({ ...stateFromView(pinned), pinned: false }, pinned, ZONE)).toEqual({
      ok: true,
      value: { floating: true },
    });
  });

  test("an empty title is refused", () => {
    expect(buildEdit({ ...same(), title: " " }, v, ZONE).ok).toBe(false);
  });
});

describe("the form's delays", () => {
  test("default overdue is no override", () => {
    expect(overdueSpec({ mode: "default" })).toEqual({ ok: true, value: null });
  });
  test("no expiries is an empty list", () => {
    expect(expirySpecs([])).toEqual({ ok: true, value: [] });
  });
});

describe("the live sentence follows the form", () => {
  test("it mentions overdue and expiry only once overridden", () => {
    const plain = summarize(summaryInput(filled({ repeat: "weekly", days: ["WE"] }), "Personal", ZONE));
    expect(plain).toBe(
      "In Personal, remind me to take the bins out every Wednesday at 18:00, at Medium priority.",
    );
    const over = summarize(
      summaryInput(
        filled({
          repeat: "weekly",
          days: ["WE"],
          overdue: { mode: "after", amount: 3, unit: "hours" },
          expiries: [newExpiry()],
        }),
        "Personal",
        ZONE,
      ),
    );
    expect(over).toContain("It goes overdue after 3 h.");
    expect(over).toContain("It expires after 1 h.");
    // Back to the defaults: gone again.
    expect(summarize(summaryInput(filled({ repeat: "weekly", days: ["WE"] }), "Personal", ZONE))).toBe(plain);
  });

  test("a half-filled form still reads", () => {
    const s = summarize(summaryInput(newState(), "Personal", ZONE));
    expect(s).toBe("In Personal, remind me to … at a time still to choose, at Medium priority.");
  });

  test("the time zone is mentioned only when it applies", () => {
    const once = summaryInput(filled({ pinned: true }), "Personal", ZONE);
    expect(once.zone).toBeNull();
    const rep = summaryInput(filled({ repeat: "daily", pinned: true }), "Personal", ZONE);
    expect(rep.zone).toBe(ZONE);
    const cdFloating = summaryInput(
      filled({ repeat: "countdown", unit: "hours", pinned: true }),
      "Personal",
      ZONE,
    );
    expect(cdFloating.zone).toBeNull();
  });
});

describe("lists", () => {
  test("an existing reminder's form starts in its list, and picking another moves it", () => {
    const v = view();
    const s = stateFromView(v);
    expect(s.listId).toBe("l1");
    expect(listMove(s, v)).toBeNull();
    expect(listMove({ ...s, listId: "l2" }, v)).toBe("l2");
    // A move is not one of the setting edits.
    const e = buildEdit({ ...s, listId: "l2" }, v, ZONE);
    expect(e.ok && Object.keys(e.value)).toEqual([]);
  });

  test("a new reminder goes in the list chosen, or the personal list if none", () => {
    expect(newReminderList(newState())).toBeNull();
    expect(newReminderList(newState("l2"))).toBe("l2");
  });
});

describe("the Pause section", () => {
  const at = (s: string) => Math.floor(new Date(s).getTime() / 1000);
  const now = at("2026-10-03T12:00:00");

  test("a new reminder isn't paused, and the sentence says nothing of it", () => {
    expect(newState().pause.mode).toBe("off");
    expect(summaryInput(filled(), "Personal", ZONE, now).paused).toBeNull();
  });

  test("an existing reminder's form starts as it is paused", () => {
    const paused = view({ pause: { from: now - 60, until: at("2026-10-20T00:00:00") } });
    expect(stateFromView(paused, now).pause).toEqual({ mode: "until", date: "2026-10-20" });
    expect(stateFromView(view(), now).pause.mode).toBe("off");
    expect(stateFromView(view({ pause: { from: 1, until: null } }), now).pause.mode).toBe("resumed");
    // A pause that has run out reads as off.
    expect(stateFromView(view({ pause: { from: 1, until: now - 1 } }), now).pause.mode).toBe("off");
  });

  test("saving asks for a pause until the day, which the sentence also says", () => {
    const v = view();
    const s = { ...stateFromView(v, now), pause: { mode: "until" as const, date: "2026-10-20" } };
    expect(buildPause(s, v, now)).toEqual({
      ok: true,
      value: { kind: "pause", until: at("2026-10-20T00:00:00") },
    });
    expect(summarize(summaryInput(s, "Personal", ZONE, now))).toMatch(/It is paused until 20 October\.$/);
    // Pausing isn't one of the edited settings.
    const edit = buildEdit(s, v, ZONE);
    expect(edit.ok && Object.keys(edit.value)).toEqual([]);
  });

  test("turning a pause off resumes early, and an untouched one sends nothing", () => {
    const v = view({ pause: { from: now - 60, until: at("2026-10-20T00:00:00") } });
    const open = stateFromView(v, now);
    expect(buildPause(open, v, now)).toEqual({ ok: true, value: { kind: "none" } });
    const off = { ...open, pause: { ...open.pause, mode: "off" as const } };
    expect(buildPause(off, v, now)).toEqual({ ok: true, value: { kind: "resume" } });
    expect(summarize(summaryInput(off, "Personal", ZONE, now))).not.toMatch(/paused/);
  });

  test("a day that has passed is refused", () => {
    const v = view();
    const s = { ...stateFromView(v, now), pause: { mode: "until" as const, date: "2026-10-03" } };
    expect(buildPause(s, v, now).ok).toBe(false);
  });
});
