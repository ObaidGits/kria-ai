import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@solidjs/testing-library";

const { approveMock, denyMock, hitlRequestMock, setCurrentRequest } = vi.hoisted(() => {
  let currentRequest: unknown = null;

  return {
    approveMock: vi.fn(async () => undefined),
    denyMock: vi.fn(async () => undefined),
    hitlRequestMock: vi.fn(() => currentRequest),
    setCurrentRequest: (request: unknown) => {
      currentRequest = request;
    },
  };
});

vi.mock("../stores/app", () => ({
  appStore: {
    hitlRequest: hitlRequestMock,
    approveAction: approveMock,
    denyAction: denyMock,
  },
}));

import HitlModal from "./HitlModal";

describe("HitlModal", () => {
  beforeEach(() => {
    approveMock.mockClear();
    denyMock.mockClear();
    hitlRequestMock.mockClear();
    setCurrentRequest({
      requestId: "approval-1",
      toolName: "delete_file",
      args: { path: "/tmp/example.txt" },
      riskLevel: "RED",
      reason: "Deleting files requires explicit approval.",
    });
  });

  it("renders approval details with a safe default action", () => {
    render(() => <HitlModal />);

    expect(
      screen.getByRole("alertdialog", { name: /review before kria continues/i })
    ).toBeInTheDocument();
    expect(screen.getByText("delete_file")).toBeInTheDocument();
    expect(screen.getByText("Deny if unsure")).toBeInTheDocument();
    expect(screen.getByText("Deleting files requires explicit approval.")).toBeInTheDocument();
  });

  it("denies the approval when Escape is pressed", async () => {
    render(() => <HitlModal />);

    await fireEvent.keyDown(window, { key: "Escape" });

    expect(denyMock).toHaveBeenCalledWith("approval-1", "User denied");
    expect(approveMock).not.toHaveBeenCalled();
  });

  it("approves only the displayed action when the approve button is clicked", async () => {
    render(() => <HitlModal />);

    await fireEvent.click(screen.getByRole("button", { name: /approve this action/i }));

    expect(approveMock).toHaveBeenCalledWith("approval-1");
    expect(denyMock).not.toHaveBeenCalled();
  });
});
