"use strict";

/**
 * oc_hash_tool — real cryptographic hashing (sha256/sha1/md5) of text or
 * base64-encoded bytes. Pure Node.js built-ins (crypto), no dependencies.
 */
const crypto = require("crypto");

module.exports = function hash_tool(args) {
  const algo = (args && args.algorithm) || "sha256";
  const allowed = ["sha256", "sha1", "md5", "sha512"];
  if (!allowed.includes(algo)) {
    throw new Error(`unsupported algorithm: ${algo} (allowed: ${allowed.join(", ")})`);
  }
  const text = args && args.text;
  if (typeof text !== "string") throw new Error("missing required parameter: text");
  const hash = crypto.createHash(algo).update(text, "utf8").digest("hex");
  return { algorithm: algo, hash };
};
