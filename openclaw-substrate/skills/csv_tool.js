"use strict";

/**
 * oc_csv_tool — parses CSV text into JSON rows, or converts JSON rows back
 * to CSV. Pure Node.js built-ins, no dependencies (air-gapped substrate).
 * Handles quoted fields with embedded commas/newlines (RFC 4180 subset).
 */
function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = "";
  let inQuotes = false;
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (inQuotes) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i++;
        continue;
      }
      field += c;
      i++;
      continue;
    }
    if (c === '"') {
      inQuotes = true;
      i++;
      continue;
    }
    if (c === ",") {
      row.push(field);
      field = "";
      i++;
      continue;
    }
    if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(field);
      field = "";
      rows.push(row);
      row = [];
      i++;
      continue;
    }
    field += c;
    i++;
  }
  if (field.length > 0 || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

function toCsvField(value) {
  const s = value === null || value === undefined ? "" : String(value);
  if (/[",\n]/.test(s)) {
    return '"' + s.replace(/"/g, '""') + '"';
  }
  return s;
}

module.exports = function csv_tool(args) {
  const mode = (args && args.mode) || "parse"; // "parse" | "to_json" | "from_json"
  if (mode === "from_json") {
    const rows = args && args.rows;
    if (!Array.isArray(rows)) throw new Error("missing required parameter: rows (array, for mode=from_json)");
    const csv = rows.map((r) => r.map(toCsvField).join(",")).join("\n");
    return { output: csv };
  }

  const text = args && args.csv;
  if (typeof text !== "string") throw new Error("missing required parameter: csv");
  const rows = parseCsv(text).filter((r) => !(r.length === 1 && r[0] === ""));

  if (mode === "to_json") {
    if (rows.length === 0) return { rows: [] };
    const header = rows[0];
    const objects = rows.slice(1).map((r) => {
      const obj = {};
      header.forEach((h, idx) => {
        obj[h] = r[idx] !== undefined ? r[idx] : "";
      });
      return obj;
    });
    return { rows: objects };
  }
  return { rows };
};
