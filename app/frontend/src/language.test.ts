// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { LanguageSupport } from "@codemirror/language";
import {
  AVAILABLE_MODES,
  MODE_LABELS,
  autoModeId,
  modeIdForExt,
  pluginModeId,
  registerPluginLanguages,
  resolveLanguage,
  syntaxModes,
  type ModeChoice,
} from "./language";
import type { SyntaxPlugin } from "./ipc";

/** True when the language's parser reports a syntax error node for `src`. */
function hasError(lang: LanguageSupport, src: string): boolean {
  let bad = false;
  lang.language.parser.parse(src).iterate({
    enter: (node) => {
      if (node.type.isError) bad = true;
    },
  });
  return bad;
}

describe("modeIdForExt", () => {
  it("maps extensions to mode ids", () => {
    const cases: Array<[string, ModeChoice | null]> = [
      ["py", "python"],
      ["PY", "python"],
      ["pyw", "python"],
      ["ts", "typescript"],
      ["tsx", "typescript"],
      ["mts", "typescript"],
      ["cts", "typescript"],
      ["js", "javascript"],
      ["mjs", "javascript"],
      ["jsx", "javascript"],
      ["yml", "yaml"],
      ["yaml", "yaml"],
      ["toml", "toml"],
      ["sh", "shell"],
      ["bash", "shell"],
      ["diff", "diff"],
      ["patch", "diff"],
      ["ps1", "powershell"],
      ["psd1", "powershell"],
      ["rs", "rust"],
      ["java", "java"],
      ["md", "markdown"],
      ["sql", "sql"],
      ["html", "html"],
      ["htm", "html"],
      ["css", "css"],
      ["xml", "xml"],
      ["svg", "xml"],
      ["plist", "xml"],
      ["log", "log"],
      ["ini", "ini"],
      ["conf", "ini"],
      ["json", "json"],
      ["c", "c"],
      ["h", "c"],
      ["hpp", "c"],
      ["cxx", "c"],
      ["cc", "c"],
    ];
    for (const [ext, mode] of cases) expect(modeIdForExt(ext), ext).toBe(mode);
  });

  it("falls back to plain text for unknown or missing extensions", () => {
    expect(modeIdForExt("txt")).toBeNull();
    expect(modeIdForExt("rsx")).toBeNull();
    expect(modeIdForExt("")).toBeNull();
  });
});

describe("resolveLanguage", () => {
  it("resolves every selectable mode (registry drift guard)", async () => {
    for (const mode of AVAILABLE_MODES) {
      if (mode === "auto" || mode === "plain") continue;
      expect(await resolveLanguage(mode), mode).toBeInstanceOf(LanguageSupport);
    }
  });

  it("returns PlainText for auto, plain and null", async () => {
    expect(await resolveLanguage(null)).toBeNull();
    expect(await resolveLanguage("auto")).toBeNull();
    expect(await resolveLanguage("plain")).toBeNull();
  });

  it("loads a language once", async () => {
    const first = await resolveLanguage("python");
    expect(await resolveLanguage("python")).toBe(first);
  });

  it("gives the TypeScript mode a TS dialect", async () => {
    const ts = await resolveLanguage("typescript");
    const js = await resolveLanguage("javascript");
    const src = "interface Foo { a: number }";
    expect(hasError(ts!, src)).toBe(false);
    expect(hasError(js!, src)).toBe(true);
  });
});

describe("MODE_LABELS", () => {
  it("covers every mode with a non-empty display name", () => {
    for (const mode of AVAILABLE_MODES) {
      expect(MODE_LABELS[mode], mode).toBeTruthy();
    }
  });
});

describe("syntax plugins (plugin interface, increment 1)", () => {
  function plugin(overrides: Partial<SyntaxPlugin> = {}): SyntaxPlugin {
    return {
      id: "plc-log",
      name: "PLC 日志",
      extensions: ["plc", "plclog"],
      keywords: ["START", "STOP"],
      lineComment: "#",
      rules: [{ regex: "^\\d{2}:\\d{2}", tag: "className" }],
      ...overrides,
    };
  }

  it("resolves registered plugins and keeps builtin priority on AUTO", async () => {
    const warnings = registerPluginLanguages([plugin()]);
    expect(warnings).toEqual([]);

    const support = await resolveLanguage(pluginModeId("plc-log"));
    expect(support).toBeInstanceOf(LanguageSupport);
    expect(() => support!.language.parser.parse("12:00 START x")).not.toThrow();

    expect(autoModeId("plc")).toBe("plugin:plc-log");
    expect(autoModeId("PLCLOG")).toBe("plugin:plc-log");
    expect(autoModeId("py")).toBe("python"); // builtins are never hijacked
    expect(await resolveLanguage(pluginModeId("nope"))).toBeNull();
  });

  it("drops rules whose regex does not compile, with a warning", () => {
    const warnings = registerPluginLanguages([
      plugin({
        id: "bad",
        name: "坏规则",
        rules: [{ regex: "([unclosed", tag: "number" }],
      }),
    ]);
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain("坏规则");
    expect(syntaxModes().some((m) => m.id === "plugin:bad")).toBe(true); // mode survives
  });

  it("lists builtin modes first, then plugin modes", () => {
    registerPluginLanguages([plugin(), plugin({ id: "second", name: "第二" })]);
    const modes = syntaxModes();
    expect(modes.slice(0, AVAILABLE_MODES.length).map((m) => m.id)).toEqual([
      ...AVAILABLE_MODES,
    ]);
    expect(modes.slice(-2).map((m) => m.label)).toEqual(["PLC 日志", "第二"]);
    registerPluginLanguages([]); // reload with none → plugins disappear
    expect(syntaxModes()).toHaveLength(AVAILABLE_MODES.length);
  });
});
