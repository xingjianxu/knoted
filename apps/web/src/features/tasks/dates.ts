import type { components } from "@knoted/api-client";

type DateValue = components["schemas"]["DateValue"];
const shanghai = new Intl.DateTimeFormat("en-GB", {
  timeZone: "Asia/Shanghai", year: "numeric", month: "2-digit", day: "2-digit",
  hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23",
});
function wallTime(instant: number): number {
  const parts = Object.fromEntries(shanghai.formatToParts(instant).map(({ type, value }) => [type, value]));
  // setUTCFullYear avoids Date.UTC's special interpretation of years 0–99.
  const date = new Date(0);
  date.setUTCFullYear(Number(parts.year), Number(parts.month) - 1, Number(parts.day));
  date.setUTCHours(Number(parts.hour), Number(parts.minute), Number(parts.second), 0);
  return date.getTime();
}
/** Resolve a Shanghai wall time only if IANA maps it to exactly one instant. */
export function dateBoundary(value: DateValue, due: boolean): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value.date) || value.timezone !== "Asia/Shanghai"
    || (value.time !== null && !/^([01]\d|2[0-3]):[0-5]\d$/.test(value.time))) return null;
  const wall = new Date(`${value.date}T${value.time ?? "00:00"}:00Z`);
  if (!Number.isFinite(wall.getTime()) || wall.toISOString().slice(0, 10) !== value.date || value.date.startsWith("0000")) return null;
  if (due && value.time === null) wall.setUTCDate(wall.getUTCDate() + 1);
  const target = wall.getTime();
  // Neighbouring days expose both offsets at a DST transition. Preserve historical
  // second offsets too; no hard-coded UTC+8 or guessed fold is used.
  const offsets = new Set([-2, -1, 0, 1, 2].map((day) => {
    const sample = target + day * 86_400_000;
    return wallTime(sample) - sample;
  }));
  const candidates = [...offsets].map((offset) => target - offset).filter((instant) => wallTime(instant) === target);
  return candidates.length === 1 ? candidates[0]! : null;
}
