/**
 * KRIA component kit — the single design-system primitive layer (design.md
 * §4.2, Req 14.4 "one component per concept"). All primitives are token-only
 * (token-lint enforced), Kobalte-backed where an accessible primitive exists,
 * ship a Storybook story, and have a focus-visible state (Req 14.5 / 17.1).
 *
 * Import from "@/kit" (or a relative path to this barrel) rather than reaching
 * into individual files, so consumers get one stable surface.
 */
export { Button } from "./Button";
export type { ButtonProps, ButtonVariant, ButtonSize } from "./Button";

export { IconButton } from "./IconButton";
export type { IconButtonProps, IconButtonVariant, IconButtonSize } from "./IconButton";

export { Input } from "./Input";
export type { InputProps } from "./Input";

export { Textarea } from "./Textarea";
export type { TextareaProps } from "./Textarea";

export { Search } from "./Search";
export type { SearchProps } from "./Search";

export { Select } from "./Select";
export type { SelectProps, SelectOption } from "./Select";

export { Card } from "./Card";
export type { CardProps } from "./Card";

export { Chip } from "./Chip";
export type { ChipProps } from "./Chip";

export { Badge } from "./Badge";
export type { BadgeProps, BadgeTone } from "./Badge";

export { StatusDot } from "./StatusDot";
export type { StatusDotProps, StatusTone } from "./StatusDot";

export { Row } from "./Row";
export type { RowProps } from "./Row";

export { Table } from "./Table";
export type { TableProps } from "./Table";

export { ProvenanceCue } from "./ProvenanceCue";
export type { ProvenanceCueProps, ProvenanceSource } from "./ProvenanceCue";

export { SegmentBar } from "./SegmentBar";
export type { SegmentBarProps, SegmentOption } from "./SegmentBar";

export { Tabs } from "./Tabs";
export type { TabsProps, TabItem } from "./Tabs";

export { Tooltip } from "./Tooltip";
export type { TooltipProps } from "./Tooltip";

export { Popover } from "./Popover";
export type { PopoverProps } from "./Popover";

export { Menu } from "./Menu";
export type { MenuProps, MenuItem } from "./Menu";

export { Dialog } from "./Dialog";
export type { DialogProps } from "./Dialog";

export { Confirm } from "./Confirm";
export type { ConfirmProps } from "./Confirm";

export { EmptyState } from "./EmptyState";
export type { EmptyStateProps } from "./EmptyState";

export { Progress } from "./Progress";
export type { ProgressProps } from "./Progress";
