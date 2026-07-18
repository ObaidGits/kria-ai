import { splitProps, type JSX, type ParentProps } from "solid-js";
import "./Table.css";

export type TableProps = ParentProps<JSX.HTMLAttributes<HTMLTableElement>>;

/** Canonical semantic table shell. Domain components own captions/columns/rows. */
export function Table(props: TableProps) {
  const [local, rest] = splitProps(props, ["class", "children"]);
  return (
    <table class={`kit-table${local.class ? ` ${local.class}` : ""}`} {...rest}>
      {local.children}
    </table>
  );
}

export default Table;
