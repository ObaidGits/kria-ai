import {
  ErrorBoundary,
  For,
  Show,
  createEffect,
  createSignal,
  onCleanup,
  onMount,
  type JSX,
} from "solid-js";
import { Badge, Button, Card, type BadgeTone } from "../../../kit";
import { t } from "../../../stores/i18n";
import {
  featureControlsStore,
  isFeatureTransitioning,
  type FeatureControl,
} from "../../../stores/featureControlsStore";

function stateTone(state: FeatureControl["state"]): BadgeTone {
  if (state === "running") return "success";
  if (state === "starting" || state === "stopping") return "warning";
  if (state === "error") return "danger";
  return "neutral";
}

function stateLabel(state: FeatureControl["state"]): string {
  return t(`settings_feature_state_${state}`);
}

function featureCopy(key: string, values: Record<string, string | number> = {}): string {
  return Object.entries(values).reduce(
    (copy, [name, value]) => copy.replaceAll(`{${name}}`, String(value)),
    t(key),
  );
}

interface SectionStateProps {
  title: string;
  description: JSX.Element;
  tone?: "neutral" | "warning" | "danger" | "success";
  role?: "status" | "alert";
  action?: JSX.Element;
}

function SectionState(props: SectionStateProps) {
  return (
    <Card
      class={`kria-settings__feature-state kria-settings__feature-state--${props.tone ?? "neutral"}`}
      role={props.role ?? "status"}
      aria-live={props.role === "alert" ? "assertive" : "polite"}
    >
      <div class="kria-settings__feature-state-copy">
        <strong>{props.title}</strong>
        <p>{props.description}</p>
      </div>
      <Show when={props.action}>
        <div class="kria-settings__feature-state-action">{props.action}</div>
      </Show>
    </Card>
  );
}

function restoreRetryFocus(
  trigger: HTMLButtonElement,
  fallback: () => HTMLButtonElement | undefined,
): void {
  queueMicrotask(() => {
    const target = trigger.isConnected && !trigger.disabled ? trigger : fallback();
    if (target?.isConnected && !target.disabled) target.focus({ preventScroll: true });
  });
}

function FeatureControlsContent(props: { setPrimaryAction: (element: HTMLButtonElement) => void }) {
  const [recovered, setRecovered] = createSignal(false);
  let primaryAction: HTMLButtonElement | undefined;
  let hadUnavailableState = false;

  createEffect(() => {
    const unavailable = Boolean(featureControlsStore.error())
      || featureControlsStore.status() === "unavailable";
    const loading = featureControlsStore.loading();

    if (unavailable) {
      hadUnavailableState = true;
      setRecovered(false);
    } else if (!loading && hadUnavailableState) {
      hadUnavailableState = false;
      setRecovered(true);
    }
  });

  const retrying = () => featureControlsStore.loading()
    && (Boolean(featureControlsStore.error()) || featureControlsStore.status() === "unavailable");
  const retry = (event: MouseEvent & { currentTarget: HTMLButtonElement }) => {
    const trigger = event.currentTarget;
    void featureControlsStore.refresh().then(() => {
      restoreRetryFocus(trigger, () => primaryAction);
    });
  };

  return (
    <section class="kria-settings__features" aria-labelledby="feature-controls-title">
      <div class="kria-settings__section-head">
        <div>
          <h2 id="feature-controls-title">{t("settings_feature_section_title")}</h2>
          <p>{t("settings_feature_section_description")}</p>
        </div>
        <Button
          ref={(element) => {
            primaryAction = element;
            props.setPrimaryAction(element);
          }}
          variant="ghost"
          size="sm"
          disabled={featureControlsStore.loading()}
          onClick={retry}
        >
          {featureControlsStore.error() || featureControlsStore.status() === "unavailable"
            ? t("settings_feature_retry")
            : t("settings_feature_refresh")}
        </Button>
      </div>

      <Show when={retrying()}>
        <SectionState
          title={t("settings_feature_retrying_title")}
          description={t("settings_feature_retrying_description")}
        />
      </Show>
      <Show when={featureControlsStore.loading() && !retrying()}>
        <SectionState
          title={featureControlsStore.controls().length > 0
            ? t("settings_feature_refreshing_title")
            : t("settings_feature_loading_title")}
          description={t("settings_feature_loading_description")}
        />
      </Show>
      <Show when={!featureControlsStore.loading() && featureControlsStore.error()}>
        {(message) => (
          <SectionState
            title={t("settings_feature_unavailable_title")}
            description={message()}
            tone="danger"
            role="alert"
            action={<Button size="sm" onClick={retry}>{t("settings_feature_retry_action")}</Button>}
          />
        )}
      </Show>
      <Show when={!featureControlsStore.loading() && !featureControlsStore.error()
        && featureControlsStore.status() === "unavailable"}>
        <SectionState
          title={t("settings_feature_unavailable_title")}
          description={t("settings_feature_unavailable_description")}
          tone="danger"
          role="alert"
          action={<Button size="sm" onClick={retry}>{t("settings_feature_retry_action")}</Button>}
        />
      </Show>
      <Show when={!featureControlsStore.loading() && !featureControlsStore.error()
        && featureControlsStore.status() === "partial"}>
        <SectionState
          title={t("settings_feature_partial_title")}
          description={featureCopy("settings_feature_partial_description", {
            valid: featureControlsStore.controls().length,
            rejected: featureControlsStore.rejectedCount(),
          })}
          tone="warning"
        />
      </Show>
      <Show when={!featureControlsStore.loading() && !featureControlsStore.error() && recovered()}>
        <SectionState
          title={t("settings_feature_recovered_title")}
          description={t("settings_feature_recovered_description")}
          tone="success"
        />
      </Show>
      <Show when={!featureControlsStore.loading() && !featureControlsStore.error()
        && featureControlsStore.status() === "empty"}>
        <SectionState
          title={t("settings_feature_empty_title")}
          description={t("settings_feature_empty_description")}
        />
      </Show>

      <Show when={featureControlsStore.controls().length > 0}>
        <div class="kria-settings__feature-grid" role="list">
          <For each={featureControlsStore.controls()}>
            {(control) => {
              const busy = () => isFeatureTransitioning(control) || featureControlsStore.isMutating(control.id);
              const detailId = `feature-control-${control.id}-detail`;
              return (
                <Card class="kria-settings__feature" role="listitem">
                  <div class="kria-settings__feature-copy">
                    <div class="kria-settings__feature-title">
                      <strong>{control.label}</strong>
                      <Badge tone={stateTone(control.state)}>{stateLabel(control.state)}</Badge>
                    </div>
                    <p>{control.description}</p>
                    <Show when={control.detail || control.error}>
                      <p
                        id={detailId}
                        classList={{ "kria-settings__feature-error": Boolean(control.error) }}
                        role={control.error ? "alert" : undefined}
                      >
                        {control.error ?? control.detail}
                      </p>
                    </Show>
                  </div>
                  <label class="kria-settings__feature-toggle" for={`feature-control-${control.id}`}>
                    <span>{control.desiredEnabled
                      ? t("settings_feature_on")
                      : t("settings_feature_off")}</span>
                    <input
                      id={`feature-control-${control.id}`}
                      class="kria-settings__feature-switch kit-focusable"
                      type="checkbox"
                      role="switch"
                      checked={control.desiredEnabled}
                      disabled={busy()}
                      aria-label={`${control.label}: ${control.desiredEnabled
                        ? t("settings_feature_on")
                        : t("settings_feature_off")}`}
                      aria-describedby={control.detail || control.error ? detailId : undefined}
                      onChange={(event) => {
                        void featureControlsStore.setEnabled(control.id, event.currentTarget.checked);
                      }}
                    />
                  </label>
                </Card>
              );
            }}
          </For>
        </div>
      </Show>
    </section>
  );
}

export function FeatureControlsSection() {
  let primaryAction: HTMLButtonElement | undefined;

  onMount(() => void featureControlsStore.initialize());
  onCleanup(() => featureControlsStore.dispose());

  return (
    <ErrorBoundary fallback={(error, reset) => {
      console.error("[FeatureControlsSection] Section render failed.", error);
      return (
        <section class="kria-settings__features" aria-labelledby="feature-controls-failure-title">
          <SectionState
            title={t("settings_feature_render_failure_title")}
            description={t("settings_feature_render_failure_description")}
            tone="danger"
            role="alert"
            action={(
              <Button
                size="sm"
                onClick={(event) => {
                  const trigger = event.currentTarget;
                  reset();
                  void featureControlsStore.refresh().then(() => {
                    restoreRetryFocus(trigger, () => primaryAction);
                  });
                }}
              >
                {t("settings_feature_retry_action")}
              </Button>
            )}
          />
          <span id="feature-controls-failure-title" class="kria-settings__feature-failure-label">
            {t("settings_feature_section_title")}
          </span>
        </section>
      );
    }}>
      <FeatureControlsContent setPrimaryAction={(element) => { primaryAction = element; }} />
    </ErrorBoundary>
  );
}

export default FeatureControlsSection;
