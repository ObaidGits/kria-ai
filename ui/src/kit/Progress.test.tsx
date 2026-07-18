import { describe, it, expect } from "vitest";
import { render } from "@solidjs/testing-library";
import { Progress } from "./Progress";

describe("Progress", () => {
  it("exposes a progressbar with the correct value attributes", () => {
    const { getByRole } = render(() => (
      <Progress label="Uploading" value={45} minValue={0} maxValue={100} />
    ));
    const bar = getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "45");
    expect(bar).toHaveAttribute("aria-valuemin", "0");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
  });

  it("renders the label text", () => {
    const { getByText } = render(() => <Progress label="Uploading" value={10} />);
    expect(getByText("Uploading")).toBeInTheDocument();
  });

  it("omits aria-valuenow when indeterminate", () => {
    const { getByRole } = render(() => <Progress label="Working" indeterminate />);
    expect(getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
  });
});
