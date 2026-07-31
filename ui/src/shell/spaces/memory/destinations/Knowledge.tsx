import type { KnowledgeProjectionItem } from "../api";
import type { SemanticScene, SceneActionKind } from "../scene/semanticScene";
import { FocusOrbit } from "../knowledge/FocusOrbit";
import "./Knowledge.css";

export type KnowledgeItem = KnowledgeProjectionItem;

export interface KnowledgeProps {
  items: KnowledgeItem[];
  scene: SemanticScene | null;
  selectedId: string | null;
  focusTrail: string[];
  loadedNodeCount: number;
  snapshotItemCount: number | null;
  graphRevision: number | null;
  snapshotTruncated: boolean;
  filterQuery: string;
  inspectorAvailable: boolean;
  pathAvailable: boolean;
  correctionAvailable: boolean;
  mapParityReady: boolean;
  isLoading: boolean;
  isSeeding?: boolean;
  error?: string | null;
  seedMessage?: string | null;
  onFilterQuery: (query: string) => void;
  onSelectItem: (id: string) => void;
  onOpenInspector: (id: string) => void;
  onRequestPath: (fromId: string, toId: string) => void;
  onSceneAction?: (itemId: string, kind: SceneActionKind) => void;
  onBack: () => void;
  onReset: () => void;
  onRetry?: () => void;
  onSeedDemo?: () => void;
}

export function Knowledge(props: KnowledgeProps) {
  function dispatchSceneAction(itemId: string, kind: SceneActionKind): void {
    if (kind === "select" || kind === "expand") props.onSelectItem(itemId);
    else props.onSceneAction?.(itemId, kind);
  }

  return (
    <section class="kria-knowledge" data-testid="knowledge-shell" aria-label="Knowledge">
      <FocusOrbit
        scene={props.scene}
        items={props.items}
        selectedId={props.selectedId}
        focusTrail={props.focusTrail}
        loadedNodeCount={props.loadedNodeCount}
        snapshotItemCount={props.snapshotItemCount}
        graphRevision={props.graphRevision}
        snapshotTruncated={props.snapshotTruncated}
        filterQuery={props.filterQuery}
        isLoading={props.isLoading}
        isSeeding={props.isSeeding}
        error={props.error}
        seedMessage={props.seedMessage}
        inspectorAvailable={props.inspectorAvailable}
        pathAvailable={props.pathAvailable}
        onFilterQuery={props.onFilterQuery}
        onAction={(event) => dispatchSceneAction(event.itemId, event.kind)}
        onOpenInspector={props.onOpenInspector}
        onRequestPath={props.onRequestPath}
        onBack={props.onBack}
        onReset={props.onReset}
        onRetry={props.onRetry}
        onSeedDemo={props.onSeedDemo}
      />
      {props.correctionAvailable && <span class="kria-knowledge__sr-status" data-testid="correction-status">Corrections available</span>}
    </section>
  );
}

export default Knowledge;