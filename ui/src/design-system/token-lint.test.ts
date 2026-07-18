import { describe, it, expect } from "vitest";
// Standalone linter runs in Node; Vitest imports pure detectors directly.
// @ts-expect-error Standalone ESM script has no generated declaration file.
import { findRawColors, findTokenDefinitions, findUndefinedTokens } from "../../scripts/token-lint.mjs";

describe("token-lint findRawColors", () => {
  it("flags 6-digit hex colors", () => {
    const f = findRawColors("color: #18a57a;");
    expect(f).toHaveLength(1);
    expect(f[0].match).toBe("#18a57a");
    expect(f[0].rule).toBe("hex-color");
    expect(f[0].line).toBe(1);
  });

  it("flags 3-digit and 8-digit hex colors", () => {
    expect(findRawColors("border: 1px solid #fff;")).toHaveLength(1);
    expect(findRawColors("background: #0c1216ff;")).toHaveLength(1);
  });

  it("flags rgb() and rgba() literals", () => {
    expect(findRawColors("color: rgb(24, 165, 122);")[0].rule).toBe("rgb-color");
    expect(findRawColors("color: rgba(24,165,122,0.2);")[0].rule).toBe("rgb-color");
  });

  it("flags hsl() and hsla() literals", () => {
    expect(findRawColors("color: hsl(160 60% 40%);")[0].rule).toBe("hsl-color");
    expect(findRawColors("color: hsla(160,60%,40%,0.5);")[0].rule).toBe("hsl-color");
  });

  it("catches a raw hex nested inside color-mix()", () => {
    const f = findRawColors("background: color-mix(in oklab, #18a57a 12%, transparent);");
    expect(f).toHaveLength(1);
    expect(f[0].match).toBe("#18a57a");
  });

  it("allows token variables and color-mix over tokens", () => {
    expect(findRawColors("color: var(--color-accent-default);")).toHaveLength(0);
    expect(
      findRawColors("background: color-mix(in oklab, var(--color-accent-default) 12%, transparent);"),
    ).toHaveLength(0);
  });

  it("allows keyword colors that are not literals", () => {
    expect(findRawColors("color: transparent;")).toHaveLength(0);
    expect(findRawColors("fill: currentColor;")).toHaveLength(0);
    expect(findRawColors("color: inherit;")).toHaveLength(0);
  });

  it("does not allow source-line bypasses", () => {
    expect(findRawColors("color: #ffffff; /* token-lint-disable */")).toHaveLength(1);
  });

  it("reports correct line numbers across multiple lines", () => {
    const text = ["a: var(--color-text-primary);", "b: #f86d6d;", "c: 12px;"].join("\n");
    const f = findRawColors(text);
    expect(f).toHaveLength(1);
    expect(f[0].line).toBe(2);
  });

  it("finds multiple literals on one line", () => {
    expect(findRawColors("border: 1px solid #fff; background: #000;")).toHaveLength(2);
  });

  it("returns nothing for clean token-only source", () => {
    const clean = [
      ".btn {",
      "  color: var(--color-text-primary);",
      "  background: var(--color-accent-default);",
      "  padding: var(--space-3);",
      "  border-radius: var(--radius-md);",
      "}",
    ].join("\n");
    expect(findRawColors(clean)).toEqual([]);
  });
});

describe("token-lint undefined-token gate", () => {
  it("finds unresolved var() references and allows declared local/generated tokens", () => {
    const definitions = findTokenDefinitions(":root { --color-text-primary: white; --local-size: 1rem; }");
    expect(findUndefinedTokens("color: var(--color-text-primary); width: var(--local-size);", definitions)).toEqual([]);
    expect(findUndefinedTokens("color: var(--color-missing);", definitions)[0]).toMatchObject({
      match: "--color-missing",
      rule: "undefined-token",
      line: 1,
    });
  });

  it("holds for generated valid and invalid token names", () => {
    const names = Array.from({ length: 100 }, (_, index) => `--generated-token-${index}`);
    const definitions = new Set(names.filter((_, index) => index % 2 === 0));
    const source = names.map((name) => `color: var(${name});`).join("\n");
    const findings = findUndefinedTokens(source, definitions);
    expect(findings.map((finding: { match: string }) => finding.match)).toEqual(
      names.filter((_, index) => index % 2 === 1),
    );
  });
});
