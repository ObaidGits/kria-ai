"use strict";

/**
 * oc_json_tool — validates and pretty-prints/minifies JSON.
 * Pure Node.js built-ins, no dependencies (air-gapped substrate).
 */
module.exports = function json_tool(args) {
  const input = args && args.json;
  const mode = (args && args.mode) || "pretty"; // "pretty" | "minify" | "validate"
  if (typeof input !== "string" || input.trim() === "") {
    throw new Error("missing required parameter: json");
  }
  let parsed;
  try {
    parsed = JSON.parse(input);
  } catch (e) {
    return { valid: false, error: e.message };
  }
  if (mode === "validate") {
    return { valid: true };
  }
  if (mode === "minify") {
    return { valid: true, output: JSON.stringify(parsed) };
  }
  return { valid: true, output: JSON.stringify(parsed, null, 2) };
};
