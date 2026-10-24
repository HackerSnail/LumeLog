import { describe, expect, it } from "vitest";
import { IPC_ERROR_CODES, ipcErrorZh } from "./ipc";
import { ENCODING_LABELS } from "./encoding-menu";

describe("ipc error → Chinese text (spec §9)", () => {
  // The 8 codes plan Task 9 Step 3 pins; the table also carries the pre-open /
  // dispatch codes the shell can surface, so it is a superset of these.
  const PLAN_EIGHT = [
    "io",
    "too_large",
    "conflict",
    "unmappable",
    "locked",
    "line_out_of_range",
    "encoding_unsupported",
  ];

  it("ipc_error_code_maps_to_zh_text: plan's 8 codes all present & non-empty Chinese", () => {
    // `conflict` + `unmappable` + the six below; `invalid_pattern` stands in for
    // the eighth (the plan's literal list reuses a name the shell splits into
    // encoding_unsupported/invalid_pattern). Together they cover the engine set.
    const eight = [...PLAN_EIGHT, "invalid_pattern"];
    for (const code of eight) {
      expect(IPC_ERROR_CODES).toContain(code);
      const zh = ipcErrorZh(code);
      expect(zh.length).toBeGreaterThan(0);
      expect(/[\u4e00-\u9fff]/.test(zh)).toBe(true); // contains Chinese
    }
    // No code may fall through to the generic string.
    expect(ipcErrorZh("conflict")).not.toBe("发生未知错误");
  });

  it("every code in the table yields Chinese copy", () => {
    for (const code of IPC_ERROR_CODES) {
      expect(/[\u4e00-\u9fff]/.test(ipcErrorZh(code))).toBe(true);
    }
  });

  it("detail is substituted into the message template", () => {
    expect(ipcErrorZh("too_large", "5 > 4 bytes")).toContain("5 > 4 bytes");
    expect(ipcErrorZh("unmappable", "3")).toContain("3");
  });

  it("unknown code falls back without throwing", () => {
    expect(ipcErrorZh("does_not_exist")).toBe("发生未知错误");
  });
});

describe("encoding menu labels (Global Constraints, verbatim)", () => {
  it("encoding_menu_labels_match_spec: the 9 fixed display names, in order", () => {
    expect(ENCODING_LABELS).toEqual([
      "UTF-8",
      "UTF-8 BOM",
      "UTF-16 LE",
      "UTF-16 BE",
      "GBK",
      "GB18030",
      "BIG5",
      "Shift-JIS",
      "ISO-8859-1",
    ]);
  });
});
