"use strict";

/**
 * oc_text_tool — common text operations: word/char/line count, case
 * conversion, trim, reverse. Pure Node.js built-ins, no dependencies
 * (air-gapped substrate).
 */
module.exports = function text_tool(args) {
  const text = args && args.text;
  const op = (args && args.op) || "stats"; // "stats" | "upper" | "lower" | "trim" | "reverse"
  if (typeof text !== "string") throw new Error("missing required parameter: text");

  switch (op) {
    case "stats": {
      const words = text.trim() === "" ? [] : text.trim().split(/\s+/);
      const lines = text.split(/\r?\n/);
      return {
        words: words.length,
        characters: text.length,
        lines: lines.length,
      };
    }
    case "upper":
      return { output: text.toUpperCase() };
    case "lower":
      return { output: text.toLowerCase() };
    case "trim":
      return { output: text.trim() };
    case "reverse":
      return { output: text.split("").reverse().join("") };
    default:
      throw new Error(`unknown op: ${op}`);
  }
};
