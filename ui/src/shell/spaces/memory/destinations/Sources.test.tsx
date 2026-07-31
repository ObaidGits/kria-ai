/**
 * Sources.test.tsx — Tests for the Sources destination component (task 4.5.2).
 */
import { describe, it, expect, vi } from 'vitest';
import { render } from '@solidjs/testing-library';
import { Sources } from './Sources';
import type {
  SourcesProps,
  SourcesState,
  Source,
  SourceDerivation,
  SourceActionPhase,
} from './Sources';

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeSource(overrides: Partial<Source> = {}): Source {
  return {
    id: 'src-1',
    label: 'My Source',
    kind: 'filesystem',
    status: 'active',
    policyLabel: 'policy-default',
    trustLevel: 'trusted',
    consentStatus: 'granted',
    version: null,
    derivations: [],
    lifecycleLabel: 'active ingestion',
    lastUpdated: '2024-01-15T10:00:00Z',
    itemCount: null,
    candidatePreview: null,
    ...overrides,
  };
}

function makeProps(
  statePatch: Partial<SourcesState> = {},
  handlers: Partial<SourcesProps> = {}
): SourcesProps {
  return {
    state: {
      sources: [],
      isLoading: false,
      errorMessage: null,
      actionPhase: { phase: 'idle' },
      ...statePatch,
    },
    onConsent: vi.fn(),
    onRevokeConsent: vi.fn(),
    onApproveCandidate: vi.fn(),
    onExcludeCandidate: vi.fn(),
    onCancel: vi.fn(),
    onResume: vi.fn(),
    onDelete: vi.fn(),
    onActionCommit: vi.fn(),
    onActionCancel: vi.fn(),
    ...handlers,
  };
}

// ─── Root renders ─────────────────────────────────────────────────────────────

describe('Sources root', () => {
  it('renders the root section', () => {
    const { getByTestId } = render(() => <Sources {...makeProps()} />);
    expect(getByTestId('sources-destination')).toBeTruthy();
  });
});

// ─── Loading state ────────────────────────────────────────────────────────────

describe('loading state', () => {
  it('shows loading indicator when isLoading=true', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ isLoading: true })} />
    );
    const el = getByTestId('sources-loading');
    expect(el).toBeTruthy();
    expect(el.getAttribute('role')).toBe('status');
  });

  it('hides loading indicator when isLoading=false', () => {
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ isLoading: false })} />
    );
    expect(queryByTestId('sources-loading')).toBeNull();
  });

  it('does not show sources-list while loading', () => {
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ isLoading: true, sources: [makeSource()] })} />
    );
    expect(queryByTestId('sources-list')).toBeNull();
  });
});

// ─── Error state ──────────────────────────────────────────────────────────────

describe('error state', () => {
  it('shows error element when errorMessage is non-null', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ errorMessage: 'Something went wrong' })} />
    );
    const el = getByTestId('sources-error');
    expect(el).toBeTruthy();
    expect(el.getAttribute('role')).toBe('alert');
    expect(el.textContent).toContain('Something went wrong');
  });

  it('hides error element when errorMessage is null', () => {
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ errorMessage: null })} />
    );
    expect(queryByTestId('sources-error')).toBeNull();
  });
});

// ─── List and empty states ────────────────────────────────────────────────────

describe('list and empty states', () => {
  it('shows sources-list when sources non-empty and not loading', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [makeSource()] })} />
    );
    const list = getByTestId('sources-list');
    expect(list).toBeTruthy();
    expect(list.getAttribute('role')).toBe('list');
  });

  it('hides sources-list when sources empty', () => {
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [] })} />
    );
    expect(queryByTestId('sources-list')).toBeNull();
  });

  it('shows sources-empty when not loading and sources empty', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [], isLoading: false })} />
    );
    expect(getByTestId('sources-empty')).toBeTruthy();
  });

  it('hides sources-empty when sources non-empty', () => {
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [makeSource()] })} />
    );
    expect(queryByTestId('sources-empty')).toBeNull();
  });

  it('hides sources-empty when loading', () => {
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [], isLoading: true })} />
    );
    expect(queryByTestId('sources-empty')).toBeNull();
  });
});

// ─── Source fields ────────────────────────────────────────────────────────────

describe('source fields', () => {
  const src = makeSource({
    id: 'src-a',
    label: 'My Filesystem',
    kind: 'filesystem',
    status: 'active',
    policyLabel: 'policy-strict',
    trustLevel: 'trusted',
    consentStatus: 'granted',
    lifecycleLabel: 'active ingestion',
    lastUpdated: '2024-06-01T12:00:00Z',
  });

  it('renders source item with correct data-testid and data-status', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    const item = getByTestId('source-src-a');
    expect(item.getAttribute('data-status')).toBe('active');
  });

  it('renders label', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-label-src-a').textContent).toBe('My Filesystem');
  });

  it('renders kind', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-kind-src-a').textContent).toBe('filesystem');
  });

  it('renders status', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-status-src-a').textContent).toBe('active');
  });

  it('renders policy label exactly', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-policy-src-a').textContent).toBe('policy-strict');
  });

  it('renders trust level', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-trust-src-a').textContent).toBe('trusted');
  });

  it('renders consent status', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-consent-status-src-a').textContent).toBe('granted');
  });

  it('renders lifecycle label', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-lifecycle-src-a').textContent).toBe('active ingestion');
  });

  it('renders lastUpdated ISO timestamp', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-last-updated-src-a').textContent).toBe('2024-06-01T12:00:00Z');
  });
});

// ─── Version conditional ──────────────────────────────────────────────────────

describe('version conditional', () => {
  it('shows version when non-null', () => {
    const src = makeSource({ id: 'v1', version: 'v2.3.1' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-version-v1').textContent).toBe('v2.3.1');
  });

  it('hides version when null', () => {
    const src = makeSource({ id: 'v2', version: null });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-version-v2')).toBeNull();
  });
});

// ─── itemCount conditional ────────────────────────────────────────────────────

describe('itemCount conditional', () => {
  it('shows itemCount when non-null', () => {
    const src = makeSource({ id: 'ic1', itemCount: 42 });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-item-count-ic1').textContent).toBe('42');
  });

  it('hides itemCount when null', () => {
    const src = makeSource({ id: 'ic2', itemCount: null });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-item-count-ic2')).toBeNull();
  });
});

// ─── Derivations ─────────────────────────────────────────────────────────────

describe('derivations', () => {
  it('shows derivations container when non-empty', () => {
    const derivations: SourceDerivation[] = [
      { derivedId: 'd-1', derivedLabel: 'Summary', derivedKind: 'summary' },
    ];
    const src = makeSource({ id: 'dr1', derivations });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-derivations-dr1')).toBeTruthy();
    expect(getByTestId('source-derivation-d-1')).toBeTruthy();
  });

  it('renders each derivation with label and kind', () => {
    const derivations: SourceDerivation[] = [
      { derivedId: 'd-2', derivedLabel: 'Skill A', derivedKind: 'skill' },
      { derivedId: 'd-3', derivedLabel: 'Rule B', derivedKind: 'rule' },
    ];
    const src = makeSource({ id: 'dr2', derivations });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    const d2 = getByTestId('source-derivation-d-2');
    expect(d2.textContent).toContain('Skill A');
    expect(d2.textContent).toContain('skill');
    const d3 = getByTestId('source-derivation-d-3');
    expect(d3.textContent).toContain('Rule B');
  });

  it('hides derivations container when empty', () => {
    const src = makeSource({ id: 'dr3', derivations: [] });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-derivations-dr3')).toBeNull();
  });
});

// ─── Candidate preview ────────────────────────────────────────────────────────

describe('candidate preview', () => {
  it('shows preview when status=candidate and candidatePreview non-null', () => {
    const src = makeSource({ id: 'cp1', status: 'candidate', candidatePreview: 'Preview text here' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    const el = getByTestId('source-candidate-preview-cp1');
    expect(el.textContent).toContain('Preview text here');
  });

  it('hides preview when status=candidate but candidatePreview is null', () => {
    const src = makeSource({ id: 'cp2', status: 'candidate', candidatePreview: null });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-candidate-preview-cp2')).toBeNull();
  });

  it('hides preview when candidatePreview non-null but status is not candidate', () => {
    const src = makeSource({ id: 'cp3', status: 'active', candidatePreview: 'Should not show' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-candidate-preview-cp3')).toBeNull();
  });
});

// ─── Candidate actions ────────────────────────────────────────────────────────

describe('candidate actions', () => {
  it('shows approve and exclude buttons for candidate sources', () => {
    const src = makeSource({ id: 'ca1', status: 'candidate' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-approve-ca1')).toBeTruthy();
    expect(getByTestId('source-exclude-ca1')).toBeTruthy();
  });

  it('hides approve/exclude for non-candidate sources', () => {
    const src = makeSource({ id: 'ca2', status: 'active' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-approve-ca2')).toBeNull();
    expect(queryByTestId('source-exclude-ca2')).toBeNull();
  });
});

// ─── Active actions ───────────────────────────────────────────────────────────

describe('active actions', () => {
  it('shows revoke-consent for active source with consentStatus != denied', () => {
    const src = makeSource({ id: 'aa1', status: 'active', consentStatus: 'granted' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-revoke-consent-aa1')).toBeTruthy();
  });

  it('hides revoke-consent for active source with consentStatus = denied', () => {
    const src = makeSource({ id: 'aa2', status: 'active', consentStatus: 'denied' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-revoke-consent-aa2')).toBeNull();
  });

  it('shows consent button for active source with consentStatus = pending', () => {
    const src = makeSource({ id: 'aa3', status: 'active', consentStatus: 'pending' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-consent-aa3')).toBeTruthy();
  });

  it('hides consent button for active source with consentStatus != pending', () => {
    const src = makeSource({ id: 'aa4', status: 'active', consentStatus: 'granted' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-consent-aa4')).toBeNull();
  });

  it('shows cancel button for active source', () => {
    const src = makeSource({ id: 'aa5', status: 'active' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-cancel-aa5')).toBeTruthy();
  });

  it('hides cancel button for non-active source', () => {
    const src = makeSource({ id: 'aa6', status: 'paused' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-cancel-aa6')).toBeNull();
  });
});

// ─── Resume action ────────────────────────────────────────────────────────────

describe('resume action', () => {
  it('shows resume button for paused source', () => {
    const src = makeSource({ id: 'ra1', status: 'paused' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-resume-ra1')).toBeTruthy();
  });

  it('shows resume button for cancelled source', () => {
    const src = makeSource({ id: 'ra2', status: 'cancelled' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-resume-ra2')).toBeTruthy();
  });

  it('hides resume button for active source', () => {
    const src = makeSource({ id: 'ra3', status: 'active' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-resume-ra3')).toBeNull();
  });
});

// ─── Delete action ────────────────────────────────────────────────────────────

describe('delete action', () => {
  it('shows delete button for active source', () => {
    const src = makeSource({ id: 'da1', status: 'active' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-delete-da1')).toBeTruthy();
  });

  it('shows delete button for paused source', () => {
    const src = makeSource({ id: 'da2', status: 'paused' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-delete-da2')).toBeTruthy();
  });

  it('shows delete button for cancelled source', () => {
    const src = makeSource({ id: 'da3', status: 'cancelled' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(getByTestId('source-delete-da3')).toBeTruthy();
  });

  it('hides delete button for completed source', () => {
    const src = makeSource({ id: 'da4', status: 'completed' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-delete-da4')).toBeNull();
  });

  it('hides delete button for candidate source', () => {
    const src = makeSource({ id: 'da5', status: 'candidate' });
    const { queryByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] })} />
    );
    expect(queryByTestId('source-delete-da5')).toBeNull();
  });
});

// ─── Callback invocation ──────────────────────────────────────────────────────

describe('callbacks', () => {
  it('calls onApproveCandidate with sourceId', async () => {
    const onApproveCandidate = vi.fn();
    const src = makeSource({ id: 'cb1', status: 'candidate' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onApproveCandidate })} />
    );
    getByTestId('source-approve-cb1').click();
    expect(onApproveCandidate).toHaveBeenCalledWith('cb1');
  });

  it('calls onExcludeCandidate with sourceId', async () => {
    const onExcludeCandidate = vi.fn();
    const src = makeSource({ id: 'cb2', status: 'candidate' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onExcludeCandidate })} />
    );
    getByTestId('source-exclude-cb2').click();
    expect(onExcludeCandidate).toHaveBeenCalledWith('cb2');
  });

  it('calls onRevokeConsent with sourceId', async () => {
    const onRevokeConsent = vi.fn();
    const src = makeSource({ id: 'cb3', status: 'active', consentStatus: 'granted' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onRevokeConsent })} />
    );
    getByTestId('source-revoke-consent-cb3').click();
    expect(onRevokeConsent).toHaveBeenCalledWith('cb3');
  });

  it('calls onConsent with sourceId', async () => {
    const onConsent = vi.fn();
    const src = makeSource({ id: 'cb4', status: 'active', consentStatus: 'pending' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onConsent })} />
    );
    getByTestId('source-consent-cb4').click();
    expect(onConsent).toHaveBeenCalledWith('cb4');
  });

  it('calls onCancel with sourceId', async () => {
    const onCancel = vi.fn();
    const src = makeSource({ id: 'cb5', status: 'active' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onCancel })} />
    );
    getByTestId('source-cancel-cb5').click();
    expect(onCancel).toHaveBeenCalledWith('cb5');
  });

  it('calls onResume with sourceId', async () => {
    const onResume = vi.fn();
    const src = makeSource({ id: 'cb6', status: 'paused' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onResume })} />
    );
    getByTestId('source-resume-cb6').click();
    expect(onResume).toHaveBeenCalledWith('cb6');
  });

  it('calls onDelete with sourceId', async () => {
    const onDelete = vi.fn();
    const src = makeSource({ id: 'cb7', status: 'active' });
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ sources: [src] }, { onDelete })} />
    );
    getByTestId('source-delete-cb7').click();
    expect(onDelete).toHaveBeenCalledWith('cb7');
  });
});

// ─── Action phase — all variants ──────────────────────────────────────────────

describe('action phase', () => {
  it('renders action phase with data-phase=idle', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: { phase: 'idle' } })} />
    );
    expect(getByTestId('source-action-phase').getAttribute('data-phase')).toBe('idle');
  });

  it('renders action phase with data-phase=confirming and shows commit/cancel buttons', () => {
    const phase: SourceActionPhase = { phase: 'confirming', sourceId: 'src-x', action: 'consent' };
    const onActionCommit = vi.fn();
    const onActionCancel = vi.fn();
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: phase }, { onActionCommit, onActionCancel })} />
    );
    expect(getByTestId('source-action-phase').getAttribute('data-phase')).toBe('confirming');
    expect(getByTestId('source-action-commit')).toBeTruthy();
    expect(getByTestId('source-action-cancel-btn')).toBeTruthy();
  });

  it('calls onActionCommit when commit button clicked', () => {
    const onActionCommit = vi.fn();
    const phase: SourceActionPhase = { phase: 'confirming', sourceId: 'src-y', action: 'delete' };
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: phase }, { onActionCommit })} />
    );
    getByTestId('source-action-commit').click();
    expect(onActionCommit).toHaveBeenCalled();
  });

  it('calls onActionCancel when cancel button clicked', () => {
    const onActionCancel = vi.fn();
    const phase: SourceActionPhase = { phase: 'confirming', sourceId: 'src-z', action: 'cancel' };
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: phase }, { onActionCancel })} />
    );
    getByTestId('source-action-cancel-btn').click();
    expect(onActionCancel).toHaveBeenCalled();
  });

  it('renders action phase with data-phase=committing', () => {
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: { phase: 'committing' } })} />
    );
    expect(getByTestId('source-action-phase').getAttribute('data-phase')).toBe('committing');
  });

  it('renders action phase with data-phase=committed and shows revision and audit', () => {
    const phase: SourceActionPhase = { phase: 'committed', newRevision: 42, auditRecordId: 'audit-99' };
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: phase })} />
    );
    expect(getByTestId('source-action-phase').getAttribute('data-phase')).toBe('committed');
    expect(getByTestId('source-action-revision').textContent).toBe('42');
    expect(getByTestId('source-action-audit').textContent).toBe('audit-99');
  });

  it('renders action phase with data-phase=error and shows alert', () => {
    const phase: SourceActionPhase = { phase: 'error', message: 'Consent failed' };
    const { getByTestId } = render(() =>
      <Sources {...makeProps({ actionPhase: phase })} />
    );
    expect(getByTestId('source-action-phase').getAttribute('data-phase')).toBe('error');
    const alert = getByTestId('source-action-error');
    expect(alert.getAttribute('role')).toBe('alert');
    expect(alert.textContent).toContain('Consent failed');
  });
});
