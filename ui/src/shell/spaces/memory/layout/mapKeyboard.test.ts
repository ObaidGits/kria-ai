/**
 * Tests for memory/layout/mapKeyboard.ts
 */
import { describe, it, expect } from 'vitest';
import { MAP_KEYS, classifyKeyEvent } from './mapKeyboard';

// ─── Constants ────────────────────────────────────────────────────────────────

describe('MAP_KEYS', () => {
  it('SELECT is Enter', () => {
    expect(MAP_KEYS.SELECT).toBe('Enter');
  });

  it('EXPAND is Enter (modifier applied at classify time)', () => {
    expect(MAP_KEYS.EXPAND).toBe('Enter');
  });

  it('ACTIONS_MENU is ContextMenu', () => {
    expect(MAP_KEYS.ACTIONS_MENU).toBe('ContextMenu');
  });

  it('ACTIONS_SHIFT_F10 is F10', () => {
    expect(MAP_KEYS.ACTIONS_SHIFT_F10).toBe('F10');
  });

  it('ESCAPE is Escape', () => {
    expect(MAP_KEYS.ESCAPE).toBe('Escape');
  });
});

// ─── classifyKeyEvent ─────────────────────────────────────────────────────────

describe('classifyKeyEvent', () => {
  // Enter variants
  it('Enter without shift → select', () => {
    expect(classifyKeyEvent('Enter', false, false)).toEqual({ action: 'select' });
  });

  it('Enter with shift → expand', () => {
    expect(classifyKeyEvent('Enter', true, false)).toEqual({ action: 'expand' });
  });

  // Actions menu
  it('ContextMenu → show-actions', () => {
    expect(classifyKeyEvent('ContextMenu', false, false)).toEqual({ action: 'show-actions' });
  });

  it('F10 + shift → show-actions', () => {
    expect(classifyKeyEvent('F10', true, false)).toEqual({ action: 'show-actions' });
  });

  it('F10 without shift → none', () => {
    expect(classifyKeyEvent('F10', false, false)).toEqual({ action: 'none' });
  });

  // Escape
  it('Escape → escape', () => {
    expect(classifyKeyEvent('Escape', false, false)).toEqual({ action: 'escape' });
  });

  // Arrow navigation
  it('ArrowUp → navigate up', () => {
    expect(classifyKeyEvent('ArrowUp', false, false)).toEqual({ action: 'navigate', direction: 'up' });
  });

  it('ArrowDown → navigate down', () => {
    expect(classifyKeyEvent('ArrowDown', false, false)).toEqual({ action: 'navigate', direction: 'down' });
  });

  it('ArrowLeft → navigate left', () => {
    expect(classifyKeyEvent('ArrowLeft', false, false)).toEqual({ action: 'navigate', direction: 'left' });
  });

  it('ArrowRight → navigate right', () => {
    expect(classifyKeyEvent('ArrowRight', false, false)).toEqual({ action: 'navigate', direction: 'right' });
  });

  // Home/End
  it('Home → home', () => {
    expect(classifyKeyEvent('Home', false, false)).toEqual({ action: 'home' });
  });

  it('End → end', () => {
    expect(classifyKeyEvent('End', false, false)).toEqual({ action: 'end' });
  });

  // Unknown keys
  it('Tab → none', () => {
    expect(classifyKeyEvent('Tab', false, false)).toEqual({ action: 'none' });
  });

  it('Space → none', () => {
    expect(classifyKeyEvent(' ', false, false)).toEqual({ action: 'none' });
  });

  it('arbitrary key → none', () => {
    expect(classifyKeyEvent('a', false, false)).toEqual({ action: 'none' });
    expect(classifyKeyEvent('F5', false, false)).toEqual({ action: 'none' });
    expect(classifyKeyEvent('Delete', false, false)).toEqual({ action: 'none' });
  });

  // ctrlKey does not affect classification
  it('ArrowUp + ctrl → still navigate up', () => {
    expect(classifyKeyEvent('ArrowUp', false, true)).toEqual({ action: 'navigate', direction: 'up' });
  });

  it('Enter + ctrl (no shift) → still select', () => {
    expect(classifyKeyEvent('Enter', false, true)).toEqual({ action: 'select' });
  });
});
