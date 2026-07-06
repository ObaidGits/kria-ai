"use strict";

/**
 * oc_regex_tool — regex match/replace/test over a text input.
 * Pure Node.js built-ins, no dependencies (air-gapped substrate).
 */
module.exports = function regex_tool(args) {
  const text = args && args.text;
  const pattern = args && args.pattern;
  const flags = (args && args.flags) || "g";
  const mode = (args && args.mode) || "match"; // "match" | "replace" | "test"
  const replacement = args && args.replacement;

  if (typeof text !== "string") throw new Error("missing required parameter: text");
  if (typeof pattern !== "string" || pattern.trim() === "") {
    throw new Error("missing required parameter: pattern");
  }

  let re;
  try {
    re = new RegExp(pattern, flags);
  } catch (e) {
    throw new Error(`invalid regular expression: ${e.message}`);
  }

  if (mode === "test") {
    return { matched: re.test(text) };
  }
  if (mode === "replace") {
    if (typeof replacement !== "string") {
      throw new Error("missing required parameter: replacement (for mode=replace)");
    }
    return { output: text.replace(re, replacement) };
  }
  const matches = text.match(re) || [];
  return { matches, count: matches.length };
};
