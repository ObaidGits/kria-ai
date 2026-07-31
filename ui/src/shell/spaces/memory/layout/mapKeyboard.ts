/**
 * memory/layout/mapKeyboard — Map composite keyboard navigation constants.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Implements the map composite keyboard behavior: one Tab entry point,
 * spatial Arrow navigation, Home/End, Enter select, Shift+Enter expand,
 * Menu/Shift+F10 actions menu, Escape nested close/focus return.
 *
 * IDs: MGR-013–016, MGR-022; MGD-013–014; MG-H11–H12.
 */

// ─── Key code constants ───────────────────────────────────────────────────────

/** Key codes for the map composite keyboard behavior. */
export const MAP_KEYS = {
  SELECT: 'Enter',
  EXPAND: 'Enter',        // + shiftKey
  ACTIONS_MENU: 'ContextMenu',
  ACTIONS_SHIFT_F10: 'F10', // + shiftKey
  ESCAPE: 'Escape',
  HOME: 'Home',
  END: 'End',
  ARROW_UP: 'ArrowUp',
  ARROW_DOWN: 'ArrowDown',
  ARROW_LEFT: 'ArrowLeft',
  ARROW_RIGHT: 'ArrowRight',
} as const;

// ─── Action discriminated union ───────────────────────────────────────────────

/** The semantic action produced by a key event on the map composite. */
export type MapKeyAction =
  | { action: 'select' }
  | { action: 'expand' }
  | { action: 'show-actions' }
  | { action: 'escape' }
  | { action: 'navigate'; direction: 'up' | 'down' | 'left' | 'right' }
  | { action: 'home' }
  | { action: 'end' }
  | { action: 'none' };

// ─── Classifier ───────────────────────────────────────────────────────────────

/**
 * Classifies a keyboard event into a semantic MapKeyAction.
 *
 * Rules (in priority order):
 *   - Enter + shiftKey → expand
 *   - Enter (no shift)  → select
 *   - ContextMenu       → show-actions
 *   - F10 + shiftKey    → show-actions
 *   - Escape            → escape
 *   - ArrowUp           → navigate up
 *   - ArrowDown         → navigate down
 *   - ArrowLeft         → navigate left
 *   - ArrowRight        → navigate right
 *   - Home              → home
 *   - End               → end
 *   - anything else     → none
 *
 * ctrlKey is accepted for future extension but does not currently affect
 * any classification (no ctrl-modified key produces a non-none action here).
 */
export function classifyKeyEvent(
  key: string,
  shiftKey: boolean,
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  _ctrlKey: boolean,
): MapKeyAction {
  switch (key) {
    case 'Enter':
      return shiftKey ? { action: 'expand' } : { action: 'select' };

    case 'ContextMenu':
      return { action: 'show-actions' };

    case 'F10':
      return shiftKey ? { action: 'show-actions' } : { action: 'none' };

    case 'Escape':
      return { action: 'escape' };

    case 'ArrowUp':
      return { action: 'navigate', direction: 'up' };

    case 'ArrowDown':
      return { action: 'navigate', direction: 'down' };

    case 'ArrowLeft':
      return { action: 'navigate', direction: 'left' };

    case 'ArrowRight':
      return { action: 'navigate', direction: 'right' };

    case 'Home':
      return { action: 'home' };

    case 'End':
      return { action: 'end' };

    default:
      return { action: 'none' };
  }
}
