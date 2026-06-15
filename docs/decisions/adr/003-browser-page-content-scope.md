# ADR: Browser Page-Content Targeting Scope (GUI Cognition v1)

Status: Accepted
Date: 2026-05-12
Owner: GUI Cognition
Scope: `crates/kria-core/src/agent/gui_cognition/browser.rs` (Task 7.2 of the
`gui-cognition-production-upgrade` spec)
Requirements: 5 (primitive/target coverage), 9 (injection-safe), 26 (English-scoped, observed-evidence-only)

## 1) Decision

Browser **web-page CONTENT** interaction is **OUT OF SCOPE for v1**:

- Clicking links / buttons **inside the rendered page** is NOT supported.
- Typing into **in-page form fields** is NOT supported.

Only browser **chrome-UI** controls (Task 7.1) are targetable: the address/URL
("omnibox") bar, the tab strip / individual tabs, back/forward, reload/stop, and
the in-page Find bar. These are REAL accessibility (AT-SPI) controls in the
browser window's accessibility tree.

Page-content interaction via a browser **DOM/CDP bridge** (e.g. Chrome DevTools
Protocol, a WebExtension content script, or a WebDriver/BiDi channel) is
**tracked as future work** and is intentionally **NOT implemented now**.

## 2) Context

Task 7.1 made browser chrome-UI targetable through the existing accessibility
resolver: a chrome control is a real a11y node with a stable role + accessible
name, so it resolves exactly like any other native control and inherits the
same trust and verification guarantees.

Web-page content is fundamentally different. Inside the rendered page, KRIA's
only observation channels are:

1. **OCR / visual-only** evidence (screenshot text + visual control detection), and
2. a **page-DOM bridge** (not built).

The accessibility tree generally does not expose arbitrary in-page DOM elements
as trustworthy, uniquely-resolvable executable controls.

## 3) Rationale — injection safety (Requirement 9)

KRIA's authority model is capability-first and verifier-aware, and it has a hard
invariant: **never execute from OCR / visual-only evidence.** The contents of a
web page are untrusted, attacker-controllable text/pixels. Resolving a click or
a keystroke target from OCR-only evidence would let page content steer the
executor — a prompt/UI-injection attack surface. Therefore:

> There are **no OCR-only page targets.** A target inside a web page is never
> resolved from OCR/visual-only evidence.

Chrome controls avoid this because they come from the accessibility tree, a
trusted execution authority, not from page pixels.

A DOM/CDP bridge *could* provide a trusted, structured page model in the future,
but it carries its own consent, sandboxing, and per-origin safety design that is
out of scope for v1. Until that bridge exists and is gated, page content stays
out of scope.

## 4) Enforcement

Behind the `gui_cog_browser` flag (default OFF until the Task 7.5 gate), when
the active app is a recognized browser:

- `classify_browser_target_scope()` / `is_page_content_target()` classify a
  target hint into **chrome-UI** (in scope) vs **page-content** (out of scope),
  using observed control **provenance**:
  - a chrome control resolves via **accessibility** → in scope;
  - a hint that does not name a chrome control (per
    `classify_browser_chrome_hint`), or that matches only **OCR/visual-only**
    controls in the page region → **page content**, out of scope.
- A page-content target is **REFUSED** with the actionable message
  (`BROWSER_PAGE_CONTENT_REFUSAL`):

  > "Web page content targeting isn't supported yet; I can act on the browser's
  > address bar, tabs, back/forward, reload, and find bar."

  rather than guessed at or acted on from OCR-only evidence.
- `resolve_browser_chrome_target()` additionally refuses to match any
  OCR/visual-only control as a chrome target (accessibility-provenance guard),
  closing the OCR-only path at resolution time as defense-in-depth.

While the flag is OFF, none of this runs and the executor/resolver path is
byte-for-byte unchanged (the prior Step 1–12 behavior).

## 5) Consequences

- **Positive:** the injection-safety boundary is preserved — page text cannot
  drive the executor. The supported surface (chrome-UI) is fully trusted and
  verifiable. The refusal is clear and redirects the user to what works.
- **Negative / limitation:** prompts like "click the Sign In button on the page"
  or "type into the page's search box" are refused in v1. Users navigate via the
  address bar, tabs, and back/forward/reload instead.
- **Future work (tracked):** a gated browser DOM/CDP bridge could bring page
  content into scope with a trusted, structured page model and its own
  consent/sandbox/per-origin safety design. This ADR is the reference point for
  that follow-up.

## 6) Alternatives Considered

- **Resolve page targets from OCR/visual evidence.** Rejected: violates the
  never-execute-from-OCR invariant (Requirement 9); opens a page-injection
  attack surface.
- **Build the DOM/CDP bridge now.** Deferred: significant consent/sandbox/
  per-origin safety surface; out of scope for v1. Tracked as future work.
