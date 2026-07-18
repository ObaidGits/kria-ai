import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import { EnrollWizardBody, type EnrollRequest, type EnrollResult } from "./EnrollWizard";

afterEach(cleanup);

function setValue(label: RegExp | string, value: string) {
  const input = screen.getByLabelText(label) as HTMLInputElement;
  fireEvent.input(input, { target: { value } });
}

/**
 * The wizard's Kobalte Dialog shell is covered by the kit Dialog tests; here we
 * exercise the step / validation / submit body directly (the modal cannot be
 * opened via a controlled prop under jsdom).
 */
describe("EnrollWizardBody — enrollment wizard (task 9.1, Req 8.1/20.4)", () => {
  it("walks Identity → Connection → Review and dispatches enrollment", async () => {
    const onEnroll = vi.fn(async (_req: EnrollRequest): Promise<EnrollResult> => ({ ok: true }));
    render(() => <EnrollWizardBody onEnroll={onEnroll} onClose={() => {}} />);

    // Step 0 — Identity
    setValue("Display name", "Office VM");
    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    // Step 1 — Connection
    setValue("Host", "192.168.1.25");
    setValue("Username", "root");
    fireEvent.click(screen.getByRole("button", { name: "Next" }));

    // Step 2 — Review → enroll
    fireEvent.click(screen.getByRole("button", { name: "Enroll device" }));

    await waitFor(() => expect(onEnroll).toHaveBeenCalledTimes(1));
    expect(onEnroll.mock.calls[0][0]).toMatchObject({
      displayName: "Office VM",
      host: "192.168.1.25",
      username: "root",
      port: 22,
    });
    await waitFor(() => expect(screen.getByText(/Device enrolled/)).toBeInTheDocument());
  });

  it("surfaces honest validation on the Connection step until host + username are set", () => {
    render(() => <EnrollWizardBody onEnroll={async () => ({ ok: true })} onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Next" })); // to Connection

    // Empty required fields are called out (never a silent block, Req 20.4).
    expect(screen.getByText("Host is required.")).toBeInTheDocument();
    expect(screen.getByText("Username is required.")).toBeInTheDocument();
  });

  it("shows a typed error when enrollment fails (honest failure, Req 20.4)", async () => {
    const onEnroll = vi.fn(
      async (): Promise<EnrollResult> => ({
        ok: false,
        title: "Connection refused",
        message: "Could not reach the host.",
      }),
    );
    render(() => <EnrollWizardBody onEnroll={onEnroll} onClose={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    setValue("Host", "10.0.0.9");
    setValue("Username", "root");
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByRole("button", { name: "Enroll device" }));

    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Connection refused"));
    expect(screen.getByText("Could not reach the host.")).toBeInTheDocument();
  });
});
