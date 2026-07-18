/**
 * Shared sanitized-markdown rendering (design.md §1.17 — SECURITY CRITICAL).
 *
 * Chat / result-card bodies may contain model- or tool-authored text. That
 * text is UNTRUSTED. This module is the single place that turns markdown into
 * HTML and it ALWAYS runs the result through DOMPurify with a strict tag/attr
 * allow-list before it reaches the DOM. No AI/tool HTML is ever rendered
 * un-sanitized. Consumers set the returned string via `innerHTML` only.
 *
 * Stack (design.md §1.17): `marked` (GFM) → `highlight.js` for fenced code →
 * `DOMPurify` sanitize. highlight.js is loaded with an explicit, bounded set of
 * languages (no auto-download) and large blocks are previewed to keep streaming
 * responsive (Req 16).
 *
 * This centralizes what the legacy `components/MessageBubble.tsx` did inline so
 * the redesign has ONE sanitizer (Req 21.2 "one component per concept").
 */
import { marked } from "marked";
import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import css from "highlight.js/lib/languages/css";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

// Large-block guards: highlighting huge blocks stalls the main thread and
// fights the streaming budget (Req 16). Preview instead of highlighting.
const CODE_HIGHLIGHT_MAX_CHARS = 80_000;
const CODE_HIGHLIGHT_MAX_LINES = 400;
const CODE_PREVIEW_HEAD_LINES = 220;
const CODE_PREVIEW_TAIL_LINES = 80;

let configured = false;

function configure(): void {
  if (configured) return;
  configured = true;

  marked.setOptions({ breaks: true, gfm: true });

  hljs.registerLanguage("bash", bash);
  hljs.registerLanguage("c", c);
  hljs.registerLanguage("cpp", cpp);
  hljs.registerLanguage("css", css);
  hljs.registerLanguage("go", go);
  hljs.registerLanguage("java", java);
  hljs.registerLanguage("javascript", javascript);
  hljs.registerLanguage("json", json);
  hljs.registerLanguage("markdown", markdown);
  hljs.registerLanguage("python", python);
  hljs.registerLanguage("rust", rust);
  hljs.registerLanguage("sql", sql);
  hljs.registerLanguage("typescript", typescript);
  hljs.registerLanguage("xml", xml);
  hljs.registerLanguage("yaml", yaml);

  const renderer = new marked.Renderer();
  renderer.code = function ({ text, lang }: { text: string; lang?: string; escaped?: boolean }) {
    const supported = lang && hljs.getLanguage(lang) ? lang : null;
    const language = supported ?? "plaintext";
    const preview = makeCodePreview(text);
    const body =
      preview.capped || !supported
        ? escapeHtml(preview.text)
        : hljs.highlight(preview.text, { language: supported }).value;
    const langLabel = escapeHtml(lang || "plaintext");
    const cappedLabel = preview.capped
      ? ` <span class="kria-md-code__limit">preview, ${preview.omittedLines} lines omitted</span>`
      : "";
    const cappedClass = preview.capped ? " kria-md-code--capped" : "";
    return (
      `<div class="kria-md-code__header${cappedClass}"><span>${langLabel}${cappedLabel}</span>` +
      `<button type="button" class="kria-md-code__copy" aria-label="Copy code block">Copy</button></div>` +
      `<pre><code class="hljs language-${language}">${body}</code></pre>`
    );
  };
  renderer.codespan = function ({ text }: { text: string }) {
    return `<code>${text}</code>`;
  };
  marked.use({ renderer });
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function makeCodePreview(text: string): { text: string; capped: boolean; omittedLines: number } {
  const lines = text.split(/\r?\n/);
  const tooLarge =
    text.length > CODE_HIGHLIGHT_MAX_CHARS || lines.length > CODE_HIGHLIGHT_MAX_LINES;
  if (!tooLarge) return { text, capped: false, omittedLines: 0 };

  const head = lines.slice(0, CODE_PREVIEW_HEAD_LINES);
  const tail = lines.slice(Math.max(lines.length - CODE_PREVIEW_TAIL_LINES, CODE_PREVIEW_HEAD_LINES));
  const omittedLines = Math.max(lines.length - head.length - tail.length, 0);
  return {
    text: [
      ...head,
      "",
      `[KRIA preview: ${omittedLines} lines omitted to keep the UI responsive. Copy the message for full content.]`,
      "",
      ...tail,
    ].join("\n"),
    capped: true,
    omittedLines,
  };
}

/**
 * The strict DOMPurify allow-list. No `script`, `style`, `iframe`, event
 * handlers (`onerror`, `onload`, …) or `javascript:` URLs survive this.
 */
const SANITIZE_CONFIG = {
  ALLOWED_TAGS: [
    "p", "br", "strong", "b", "em", "i", "del", "a", "code", "pre", "div",
    "h1", "h2", "h3", "h4", "h5", "h6",
    "ul", "ol", "li", "blockquote", "table", "thead", "tbody",
    "tr", "th", "td", "hr", "span", "button", "img",
  ],
  ALLOWED_ATTR: ["href", "target", "rel", "class", "src", "alt", "type", "aria-label"],
};

/**
 * Render untrusted markdown to a sanitized HTML string. ALWAYS sanitized —
 * callers may safely assign the result to `innerHTML`.
 */
export function renderMarkdown(content: string): string {
  configure();
  const raw = marked.parse(content ?? "", { async: false }) as string;
  return DOMPurify.sanitize(raw, SANITIZE_CONFIG) as unknown as string;
}

/**
 * Sanitize an already-HTML fragment (e.g. a tool result that emits HTML) with
 * the same strict policy. Plain text passes through unchanged.
 */
export function sanitizeHtml(html: string): string {
  return DOMPurify.sanitize(html ?? "", SANITIZE_CONFIG) as unknown as string;
}
