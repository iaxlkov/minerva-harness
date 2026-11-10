// assistant-ui Elements composer: https://www.assistant-ui.com/elements/composer
// Utility names adapted to this project's Tailwind 3 prefix.
import type {ComponentProps} from "react";
import {ArrowUpIcon, PlusIcon, SquareIcon, ChevronDownIcon} from "lucide-react";
import {cn} from "../../../lib/utils";
import {paper, ghostButton, inkButton, iconSwap, iconSwapIn, iconSwapOut} from "./surfaces";

export function ComposerBar({
  dragActive = false,
  className,
  ...props
}: ComponentProps<"div"> & { dragActive?: boolean }) {
  return (
    <div
      data-slot="composer-bar"
      data-drag-active={dragActive || undefined}
      className={cn(
        paper,
        "oe-flex oe-w-full oe-flex-col oe-gap-2 oe-rounded-[24px] oe-p-2.5 oe-transition-colors",
        dragActive && "oe-bg-blue-500/[0.04] dark:oe-bg-blue-500/10",
        className,
      )}
      {...props}
    />
  );
}

export function ComposerToolbar({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      data-slot="composer-toolbar"
      className={cn("oe-flex oe-items-center oe-justify-between", className)}
      {...props}
    />
  );
}

export function ComposerActions({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      data-slot="composer-actions"
      className={cn("oe-flex oe-items-center oe-gap-1.5", className)}
      {...props}
    />
  );
}

export function ComposerAttachButton({
  className,
  ...props
}: Omit<ComponentProps<"button">, "children">) {
  return (
    <button
      type="button"
      aria-label="Add attachment"
      data-slot="composer-attach"
      disabled={!props.onClick}
      className={cn(
        ghostButton,
        "oe-size-8 disabled:oe-pointer-events-none disabled:oe-opacity-30",
        className,
      )}
      {...props}
    >
      <PlusIcon className="oe-size-4" />
    </button>
  );
}

export function ComposerModelTrigger({
  model,
  open,
  className,
  ...props
}: Omit<ComponentProps<"button">, "children"> & {
  model: string;
  open?: boolean;
}) {
  return (
    <button
      type="button"
      aria-expanded={open}
      data-slot="composer-model-trigger"
      className={cn(
        "oe-text-muted-foreground hover:oe-bg-[color:color-mix(in_srgb,var(--foreground)_6%,transparent)] hover:oe-text-[color:color-mix(in_srgb,var(--foreground)_90%,transparent)] dark:hover:oe-bg-[color:color-mix(in_srgb,var(--foreground)_9%,transparent)] oe-flex oe-h-8 oe-items-center oe-gap-1.5 oe-rounded-full oe-px-3 oe-text-[12.5px] oe-transition-colors",
        className,
      )}
      {...props}
    >
      {model}
      <ChevronDownIcon className="oe-size-3 oe-opacity-60" />
    </button>
  );
}

export function ComposerSend({
  streaming,
  idle,
  className,
  ...props
}: Omit<ComponentProps<"button">, "children"> & {
  streaming: boolean;
  idle: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={streaming ? "Stop generating" : "Send message"}
      data-slot="composer-send"
      className={cn(
        "oe-grid oe-size-8 oe-place-items-center oe-rounded-full",
        streaming || !idle
          ? inkButton
          : "oe-bg-[color:color-mix(in_srgb,var(--foreground)_6%,transparent)] oe-text-muted-foreground oe-ring-inset oe-ring-border dark:oe-bg-[color:color-mix(in_srgb,var(--foreground)_9%,transparent)] oe-ring-1 oe-transition-colors forced-colors:oe-border forced-colors:oe-border-[GrayText] forced-colors:oe-text-[GrayText]",
        className,
      )}
      {...props}
    >
      <ArrowUpIcon
        className={cn(iconSwap, "oe-size-4", streaming ? iconSwapOut : iconSwapIn)}
      />
      <SquareIcon
        className={cn(
          iconSwap,
          "oe-size-3 oe-fill-current",
          streaming ? iconSwapIn : iconSwapOut,
        )}
      />
    </button>
  );
}
