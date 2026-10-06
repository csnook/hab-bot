import { describe, expect, test } from "vitest";
import { coordinateText, describePlace, parseCoordinates } from "./home";

describe("the home location", () => {
  test("latitude and longitude are read as typed", () => {
    expect(parseCoordinates("51.5074", " -0.1278 ")).toEqual({
      ok: true,
      value: { latitude: 51.5074, longitude: -0.1278 },
    });
    expect(parseCoordinates("-33", "151")).toMatchObject({ ok: true });
    expect(parseCoordinates("90", "180")).toMatchObject({ ok: true });
  });

  test("things that aren't degrees are refused", () => {
    for (const [a, b] of [
      ["", "0"],
      ["abc", "0"],
      ["51,5", "0"],
      ["0", ""],
      ["91", "0"],
      ["0", "-180.5"],
      ["1e3", "0"],
      ["NaN", "0"],
    ]) {
      expect(parseCoordinates(a, b).ok).toBe(false);
    }
  });

  test("a place reads as north or south, east or west", () => {
    expect(describePlace({ latitude: 51.5074, longitude: -0.1278 })).toBe("51.5074° N, 0.1278° W");
    expect(describePlace({ latitude: -33.8688, longitude: 151.2093 })).toBe("33.8688° S, 151.2093° E");
  });

  test("a field shows no stray digits", () => {
    expect(coordinateText(51.5074)).toBe("51.5074");
    expect(coordinateText(0.1 + 0.2)).toBe("0.3");
  });
});
