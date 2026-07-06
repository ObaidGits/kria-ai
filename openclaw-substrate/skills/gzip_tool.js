"use strict";

/**
 * oc_gzip_tool — real gzip compress/decompress of a text string, base64
 * encoded for transport. Pure Node.js built-ins (zlib), no dependencies.
 *
 * Honest scope note: this is gzip (single-stream), not a multi-file ZIP
 * archive format — Node's standard library has no built-in ZIP writer, and
 * this air-gapped substrate installs no npm packages at runtime. Real
 * single-file compression is fully functional; a true multi-file .zip would
 * require a real dependency, which the frozen air-gapped image design
 * (no npm in the final image) intentionally excludes.
 */
const zlib = require("zlib");

module.exports = function gzip_tool(args) {
  const mode = (args && args.mode) || "compress"; // "compress" | "decompress"
  if (mode === "compress") {
    const text = args && args.text;
    if (typeof text !== "string") throw new Error("missing required parameter: text");
    const compressed = zlib.gzipSync(Buffer.from(text, "utf8"));
    return {
      base64: compressed.toString("base64"),
      original_bytes: Buffer.byteLength(text, "utf8"),
      compressed_bytes: compressed.length,
    };
  }
  if (mode === "decompress") {
    const base64 = args && args.base64;
    if (typeof base64 !== "string") throw new Error("missing required parameter: base64");
    const decompressed = zlib.gunzipSync(Buffer.from(base64, "base64"));
    return { text: decompressed.toString("utf8") };
  }
  throw new Error(`unknown mode: ${mode}`);
};
