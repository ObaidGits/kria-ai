/**
 * Tests for memory/layout/visualAliases.ts
 */
import { describe, it, expect } from 'vitest';
import type { SceneItemKind } from '../scene/semanticScene';
import { VISUAL_ALIASES, getVisualAlias } from './visualAliases';

// All 10 SceneItemKind values
const ALL_KINDS: SceneItemKind[] = [
  'entity',
  'memory',
  'relation',
  'goal',
  'source',
  'episode',
  'summary',
  'skill',
  'rule',
  'navigation-container',
];

// ─── Coverage ─────────────────────────────────────────────────────────────────

describe('VISUAL_ALIASES coverage', () => {
  it('has an entry for all 10 SceneItemKind values', () => {
    for (const kind of ALL_KINDS) {
      expect(VISUAL_ALIASES[kind]).toBeDefined();
    }
  });

  it('getVisualAlias returns non-null for all 10 kinds', () => {
    for (const kind of ALL_KINDS) {
      expect(getVisualAlias(kind)).not.toBeNull();
      expect(getVisualAlias(kind)).not.toBeUndefined();
    }
  });
});

// ─── textFallback ─────────────────────────────────────────────────────────────

describe('textFallback', () => {
  it('every alias has a non-empty textFallback', () => {
    for (const kind of ALL_KINDS) {
      const alias = getVisualAlias(kind);
      expect(typeof alias.textFallback).toBe('string');
      expect(alias.textFallback.length).toBeGreaterThan(0);
    }
  });
});

// ─── colorToken format ────────────────────────────────────────────────────────

describe('colorToken', () => {
  it('all color tokens start with --color-', () => {
    for (const kind of ALL_KINDS) {
      const alias = getVisualAlias(kind);
      expect(alias.colorToken).toMatch(/^--color-/);
    }
  });

  it('each kind has a distinct colorToken', () => {
    const tokens = ALL_KINDS.map((k) => getVisualAlias(k).colorToken);
    const unique = new Set(tokens);
    expect(unique.size).toBe(ALL_KINDS.length);
  });
});

// ─── iconId ───────────────────────────────────────────────────────────────────

describe('iconId', () => {
  it('all icon IDs are non-empty strings', () => {
    for (const kind of ALL_KINDS) {
      const alias = getVisualAlias(kind);
      expect(typeof alias.iconId).toBe('string');
      expect(alias.iconId.length).toBeGreaterThan(0);
    }
  });
});

// ─── navigation-container specifics ──────────────────────────────────────────

describe('navigation-container', () => {
  it('has authorityIndicator: none', () => {
    const alias = getVisualAlias('navigation-container');
    expect(alias.authorityIndicator).toBe('none');
  });

  it('has a textFallback', () => {
    const alias = getVisualAlias('navigation-container');
    expect(alias.textFallback.length).toBeGreaterThan(0);
  });
});

// ─── Authority indicators ─────────────────────────────────────────────────────

describe('authorityIndicator', () => {
  it('only navigation-container has none', () => {
    for (const kind of ALL_KINDS) {
      if (kind === 'navigation-container') {
        expect(getVisualAlias(kind).authorityIndicator).toBe('none');
      } else {
        expect(getVisualAlias(kind).authorityIndicator).not.toBe('none');
      }
    }
  });

  it('relation uses line authority indicator', () => {
    expect(getVisualAlias('relation').authorityIndicator).toBe('line');
  });
});

// ─── Shape assignments ────────────────────────────────────────────────────────

describe('shape assignments', () => {
  it('entity is circle', () => {
    expect(getVisualAlias('entity').shape).toBe('circle');
  });

  it('relation is line', () => {
    expect(getVisualAlias('relation').shape).toBe('line');
  });

  it('goal is diamond', () => {
    expect(getVisualAlias('goal').shape).toBe('diamond');
  });

  it('rule is triangle', () => {
    expect(getVisualAlias('rule').shape).toBe('triangle');
  });

  it('source is hexagon', () => {
    expect(getVisualAlias('source').shape).toBe('hexagon');
  });

  it('skill is hexagon', () => {
    expect(getVisualAlias('skill').shape).toBe('hexagon');
  });
});

// ─── getVisualAlias matches VISUAL_ALIASES ────────────────────────────────────

describe('getVisualAlias', () => {
  it('returns the same object as VISUAL_ALIASES[kind]', () => {
    for (const kind of ALL_KINDS) {
      expect(getVisualAlias(kind)).toBe(VISUAL_ALIASES[kind]);
    }
  });
});
