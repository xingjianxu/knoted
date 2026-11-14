import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(
  ...classes: Array<string | false | null | undefined>
): string {
  return twMerge(clsx(classes));
}
