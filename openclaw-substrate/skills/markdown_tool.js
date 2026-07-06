"use strict";

/**
 * oc_markdown_tool — converts a small, common subset of Markdown to HTML
 * (headings, bold, italic, links, unordered lists, paragraphs, code spans).
 * Pure Node.js built-ins, no dependencies (air-gapped substrate). Not a full
 * CommonMark implementation — intentionally scoped to what a sandboxed
 * substrate skill can safely and predictably render without a parser
 * dependency.
 */
function escapeHtml(s) {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function inline(text) {
  let out = escapeHtml(text);
  out = out.replace(/`([^`]+)`/g, "<code>$1</code>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  out = out.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>');
  return out;
}

module.exports = function markdown_tool(args) {
  const md = args && args.markdown;
  if (typeof md !== "string") throw new Error("missing required parameter: markdown");

  const lines = md.split(/\r?\n/);
  const html = [];
  let inList = false;

  for (const line of lines) {
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      if (inList) {
        html.push("</ul>");
        inList = false;
      }
      const level = heading[1].length;
      html.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }
    const listItem = line.match(/^[-*]\s+(.*)$/);
    if (listItem) {
      if (!inList) {
        html.push("<ul>");
        inList = true;
      }
      html.push(`<li>${inline(listItem[1])}</li>`);
      continue;
    }
    if (inList) {
      html.push("</ul>");
      inList = false;
    }
    if (line.trim() === "") continue;
    html.push(`<p>${inline(line)}</p>`);
  }
  if (inList) html.push("</ul>");

  return { html: html.join("\n") };
};
