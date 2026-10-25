// Pure timeline-axis selection for the query panel (M3 increment 2 spec §2).
// Kept separate from `main.ts` so the extract-vs-line-range decision is unit
// testable without DOM. No IPC import beyond types.

import type { TimeFormat, TimeSpec } from "./ipc";

export interface TimelineSelection {
  time?: TimeSpec;
  lineFrom?: number;
  lineTo?: number;
  error?: string;
}

/**
 * Turn the panel's raw timeline inputs into a backend `filterRun` selection.
 * A non-empty extract regex selects the TIME axis (from/to are timestamp strings
 * the backend parses with `format`; either side may be blank = open bound).
 * Otherwise a filled [from,to] degrades to a 1-based LINE range — both sides
 * required (backend keeps a range only as a closed pair), positive integers,
 * from <= to. Fully blank = no timeline axis (empty selection). Returns an
 * `error` string instead of throwing so the caller can surface it as a toast.
 */
export function pickTimelineInput(
  extractPattern: string,
  format: TimeFormat,
  from: string,
  to: string,
  keepWhenNoTs: boolean,
): TimelineSelection {
  const pattern = extractPattern.trim();
  if (pattern !== "") {
    const f = from.trim();
    const t = to.trim();
    return {
      time: {
        extractPattern: pattern,
        format,
        from: f === "" ? undefined : f,
        to: t === "" ? undefined : t,
        keepWhenNoTs,
      },
    };
  }

  const f = from.trim();
  const t = to.trim();
  if (f === "" && t === "") return {};
  if (f === "" || t === "") return { error: "行号区间需同时给出起止行" };

  const nf = Number(f);
  const nt = Number(t);
  if (!Number.isInteger(nf) || !Number.isInteger(nt) || nf < 1 || nt < 1) {
    return { error: "行号需为正整数" };
  }
  if (nf > nt) return { error: "起始行号不能大于结束行号" };
  return { lineFrom: nf, lineTo: nt };
}
