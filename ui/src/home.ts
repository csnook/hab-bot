import type { Place } from "./api";
import type { Built } from "./delays";

/**
 * The home location in Settings → You and the prompt the editor makes the
 * first time a reminder needs it. Every device accepts a latitude and a
 * longitude; "Use where I am now" is for phones, which this app doesn't run
 * on yet.
 */

export interface Coordinates {
  latitude: number;
  longitude: number;
}

const NUMBER = /^[+-]?\d+(\.\d+)?$/;

/** Degrees as typed, "51.5074" and "-0.1278", checked against their ranges. */
export function parseCoordinates(lat: string, lon: string): Built<Coordinates> {
  const a = lat.trim();
  const b = lon.trim();
  if (!NUMBER.test(a)) return { ok: false, error: "Latitude is a number such as 51.5074." };
  if (!NUMBER.test(b)) return { ok: false, error: "Longitude is a number such as -0.1278." };
  const latitude = Number(a);
  const longitude = Number(b);
  if (latitude < -90 || latitude > 90) {
    return { ok: false, error: "Latitude is between -90 and 90 degrees." };
  }
  if (longitude < -180 || longitude > 180) {
    return { ok: false, error: "Longitude is between -180 and 180 degrees." };
  }
  return { ok: true, value: { latitude, longitude } };
}

/** "51.5074° N, 0.1278° W". */
export function describePlace(p: Pick<Place, "latitude" | "longitude">): string {
  const ns = p.latitude >= 0 ? "N" : "S";
  const ew = p.longitude >= 0 ? "E" : "W";
  const f = (n: number) => String(Math.round(Math.abs(n) * 10000) / 10000);
  return `${f(p.latitude)}° ${ns}, ${f(p.longitude)}° ${ew}`;
}

/** The text for a coordinate field: no trailing zeros. */
export const coordinateText = (n: number) => String(Math.round(n * 1e6) / 1e6);
