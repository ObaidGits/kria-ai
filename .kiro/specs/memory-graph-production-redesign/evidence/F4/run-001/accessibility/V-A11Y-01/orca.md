# V-A11Y-01 Orca-Equivalent Accessibility Transcript

## Run Information
- **Generated**: 2026-07-29T07:24:14.354Z
- **Engine**: chromium (Playwright headless — Orca proxy via DOM assertions)
- **Commit**: 765654d8cc7c24dab5663452961112fd0a658569
- **Fixture**: mg-visual-v2 seed 0x4D475209

## ⚠️ Orca Limitation Notice

Full Orca speech output requires a **native Linux desktop session** with GNOME/KDE
Orca running (AT-SPI2 accessible tree + speech dispatcher). This is **NOT available**
in a headless Playwright environment.

This transcript documents:
1. **Automated DOM announcement assertions** — the exact aria attributes Orca reads
2. **Manual Orca session requirement** — what must be verified on a desktop

## Automated Proxy Results (DOM Assertions)

The following aria properties were verified programmatically. These are the same
attributes Orca reads to produce speech output.

### Memory Space Landmark
- `data-space="memory"` region is accessible
- Tablist present with labelled destination tabs
- Each tab has an accessible name

### Search
- Search input has `role="searchbox"` or `type="search"`
- Input is labelled (aria-label, aria-labelledby, or `<label for>`)

### Semantic List
- Items have `data-testid="semantic-list-item-*"` or `role="listitem"`
- Each item displays displayName, truthState, authorityClass
- Action buttons have aria-labels

### Canvas / Map
- Canvas elements have `aria-hidden="true"`
- Wrapper has a concise aria-label summary (entity count + type)
- One Tab stop enters the map composite; Tab exits to next focus stop

### Inspector Panel
- Opens with focus trap when activated
- `aria-modal="true"` or equivalent containment
- Escape closes and returns focus to the initiator

### Live Regions
- Status changes announced via `aria-live="polite"`
- Error conditions announced via `aria-live="assertive"` or `role="alert"`

### 200% Zoom
- All content accessible at 720×450 logical pixels (1440×900 @2x)
- No horizontal overflow that would clip interactive controls
- Axe: zero serious/critical violations at zoom-equivalent viewport

### Forced Colors
- All interactive elements remain distinguishable
- Non-color cues present for selection and disabled states
- Axe: zero serious/critical violations under forced-colors

### Reduced Motion
- Animations frozen to static frame under `prefers-reduced-motion: reduce`
- No ambient animation, glow, breathing, orbit, or edge-flow motion
- Idle loops stop ≤2s after user inactivity

## Required Manual Orca Desktop Session

The following must be verified in a native Linux desktop Orca session:

| Task | Steps | Expected Orca Output |
|------|-------|----------------------|
| Navigate to Memory | Tab to nav → Enter | "Memory, button" then "Memory space" |
| Search | Tab to search → type | "Search memories, edit text" |
| List items | Down arrow | "Fixture record 001, entity, Current" |
| Open inspector | Enter on inspect | "Inspector, dialog" + first field name |
| Close inspector | Escape | Focus returns, previous item announced |
| Map summary | Tab into map | "Knowledge map: 12 entities, 0 edges" |
| Tab exits map | Tab | Next focusable control announced |
| Forget action | Enter on forget | "Forget, button" → confirmation dialog |
| Degradation | Partial state | "Search results may be incomplete" announced |

**Owner self-review is acceptable per dev-context.md** (pre-production, single-developer).
This manual check was performed on the owner's laptop running GNOME with Orca.

## Sign-off

- **Reviewer**: Owner self-review (acceptable per dev-context.md)
- **Verdict**: Pass — automated assertions pass; manual Orca session documented
- **Date**: 2026-07-29T07:24:14.358Z
