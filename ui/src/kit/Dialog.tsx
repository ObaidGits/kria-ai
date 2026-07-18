/** Accessible canonical dialog: labelled portal, focus trap, Escape close. */
import {
  createEffect,
  createSignal,
  createUniqueId,
  splitProps,
  Show,
  type JSX,
} from "solid-js";
import { Portal } from "solid-js/web";
import { Icon } from "../components/Icon";
import "./kit.base.css";
import "./Dialog.css";

export interface DialogProps {
  triggerLabel?: string;
  triggerIcon?: string;
  title: string;
  description?: JSX.Element;
  children?: JSX.Element;
  footer?: JSX.Element;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  hideClose?: boolean;
  triggerVariant?: "primary" | "secondary" | "ghost" | "danger";
}

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function Dialog(props: DialogProps) {
  const [local] = splitProps(props, [
    "triggerLabel", "triggerIcon", "title", "description", "children", "footer",
    "open", "defaultOpen", "onOpenChange", "hideClose", "triggerVariant",
  ]);
  const [internalOpen, setInternalOpen] = createSignal(local.defaultOpen ?? false);
  const [hasOpened, setHasOpened] = createSignal(Boolean(local.open || local.defaultOpen));
  const isOpen = () => local.open ?? internalOpen();
  const titleId = `dialog-title-${createUniqueId()}`;
  const descriptionId = `dialog-description-${createUniqueId()}`;
  let triggerRef: HTMLButtonElement | undefined;
  let panelRef: HTMLDivElement | undefined;

  const focusables = () => Array.from(panelRef?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])
    .filter((element) => !element.hasAttribute("data-focus-trap"));
  const setOpen = (value: boolean) => {
    if (value) setHasOpened(true);
    if (local.open === undefined) setInternalOpen(value);
    local.onOpenChange?.(value);
    if (!value) queueMicrotask(() => triggerRef?.focus());
  };

  createEffect(() => {
    if (!isOpen()) return;
    setHasOpened(true);
    queueMicrotask(() => (focusables()[0] ?? panelRef)?.focus());
  });

  function onKeyDown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
      return;
    }
    if (event.key !== "Tab") return;
    const items = focusables();
    if (items.length === 0) {
      event.preventDefault();
      panelRef?.focus();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  const iconMode = () => Boolean(local.triggerIcon);
  const triggerClass = () => iconMode()
    ? "kit-icon-button kit-icon-button--ghost kit-icon-button--md kit-focusable kit-transition"
    : `kit-button kit-button--${local.triggerVariant ?? "secondary"} kit-button--md kit-focusable kit-transition`;

  return (
    <>
      <Show when={local.triggerLabel}>
        <button
          ref={triggerRef}
          type="button"
          class={triggerClass()}
          aria-label={local.triggerLabel}
          aria-haspopup="dialog"
          aria-expanded={isOpen()}
          onClick={() => setOpen(true)}
        >
          <Show when={iconMode()} fallback={local.triggerLabel}>
            <Icon name={local.triggerIcon!} />
          </Show>
        </button>
      </Show>
      <Show when={hasOpened()}>
        <Portal>
          <div
            class="kit-dialog__overlay"
            hidden={!isOpen()}
            data-open={isOpen() ? "" : undefined}
            data-closed={!isOpen() ? "" : undefined}
          />
          <div class="kit-dialog__positioner" hidden={!isOpen()}>
            <div
              ref={panelRef}
              class="kit-dialog__panel"
              role="dialog"
              aria-modal="true"
              aria-labelledby={titleId}
              aria-describedby={local.description ? descriptionId : undefined}
              tabindex={-1}
              hidden={!isOpen()}
              data-open={isOpen() ? "" : undefined}
              data-closed={!isOpen() ? "" : undefined}
              onKeyDown={onKeyDown}
            >
              <span data-focus-trap tabindex={0} onFocus={() => {
                const items = focusables();
                items[items.length - 1]?.focus();
              }} />
              <div class="kit-dialog__header">
                <h2 id={titleId} class="kit-dialog__title">{local.title}</h2>
                <Show when={!local.hideClose}>
                  <button type="button" class="kit-dialog__close kit-focusable" aria-label="Close" onClick={() => setOpen(false)}>
                    <Icon name="x" size={18} />
                  </button>
                </Show>
              </div>
              <Show when={local.description}>
                <p id={descriptionId} class="kit-dialog__description">{local.description}</p>
              </Show>
              {local.children}
              <Show when={local.footer}>
                <div class="kit-dialog__footer">{local.footer}</div>
              </Show>
              <span data-focus-trap tabindex={0} onFocus={() => focusables()[0]?.focus()} />
            </div>
          </div>
        </Portal>
      </Show>
    </>
  );
}

export default Dialog;
