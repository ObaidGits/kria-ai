import { render, screen } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import { Table } from "./Table";

describe("Table — canonical semantic table (Req 14.4/17.2)", () => {
  it("renders native table semantics and preserves domain class names", () => {
    render(() => (
      <Table class="domain-table">
        <caption>Devices</caption>
        <tbody><tr><td>Local</td></tr></tbody>
      </Table>
    ));
    expect(screen.getByRole("table", { name: "Devices" })).toHaveClass("kit-table", "domain-table");
    expect(screen.getByRole("cell", { name: "Local" })).toBeInTheDocument();
  });
});
