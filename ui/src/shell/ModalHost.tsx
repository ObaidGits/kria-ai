/**
 * ModalHost — renders the single active modal (Req 1.6). Backed by the kit
 * Dialog (Kobalte) so focus-trap, Escape-to-close, and labelling come for free
 * (Req 17.6). Only ever renders ONE modal because `modalHost` refuses to open a
 * second while one is active.
 *
 * Requirements: 1.6, 17.6
 */
import { Show } from "solid-js";
import { Dialog } from "../kit";
import { modalHost, closeModal } from "./modalHost";

export function ModalHost() {
  return (
    <Show when={modalHost.activeModal()}>
      {(modal) => (
        <Dialog
          open={true}
          title={modal().title}
          description={modal().description}
          footer={modal().footer}
          hideClose={modal().hideClose}
          onOpenChange={(next) => {
            if (!next) closeModal(modal().id);
          }}
        >
          {modal().render()}
        </Dialog>
      )}
    </Show>
  );
}

export default ModalHost;
