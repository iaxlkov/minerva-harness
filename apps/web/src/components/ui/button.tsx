import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";
import { Slot } from "@radix-ui/react-slot";
const buttonVariants = cva(
  "oe-group/button oe-inline-flex oe-shrink-0 oe-items-center oe-justify-center oe-rounded-md oe-border oe-border-solid oe-bg-clip-padding oe-text-xs oe-leading-relaxed oe-font-medium oe-whitespace-nowrap oe-transition-all oe-outline-none oe-select-none focus-visible:oe-border-ring focus-visible:oe-ring-2 focus-visible:oe-ring-ring/30 active:[&:not([aria-haspopup])]:oe-translate-y-px disabled:oe-pointer-events-none disabled:oe-opacity-50 aria-invalid:oe-border-destructive aria-invalid:oe-ring-2 aria-invalid:oe-ring-destructive/20 dark:aria-invalid:oe-border-destructive/50 dark:aria-invalid:oe-ring-destructive/40 [&_svg]:oe-pointer-events-none [&_svg]:oe-shrink-0 [&_svg:not([class*='size-'])]:oe-size-4",
  {
    variants: {
      variant: {
        default:
          "oe-border-transparent oe-bg-primary oe-text-primary-foreground hover:oe-bg-primary/80",
        outline:
          "oe-border-border oe-bg-transparent oe-text-foreground hover:oe-bg-input/50 hover:oe-text-foreground aria-expanded:oe-bg-muted aria-expanded:oe-text-foreground dark:oe-bg-input/30",
        secondary:
          "oe-bg-secondary oe-text-secondary-foreground hover:oe-bg-[color-mix(in_oklch,var(--secondary),var(--foreground)_5%)] aria-expanded:oe-bg-secondary aria-expanded:oe-text-secondary-foreground",
        ghost:
          "oe-border-transparent oe-bg-transparent oe-text-foreground hover:oe-bg-muted hover:oe-text-foreground aria-expanded:oe-bg-muted aria-expanded:oe-text-foreground dark:hover:oe-bg-muted/50",
        destructive:
          "oe-bg-destructive/10 oe-text-destructive hover:oe-bg-destructive/20 focus-visible:oe-border-destructive/40 focus-visible:oe-ring-destructive/20 dark:oe-bg-destructive/20 dark:hover:oe-bg-destructive/30 dark:focus-visible:oe-ring-destructive/40",
        link: "oe-text-primary oe-underline-offset-4 hover:oe-underline",
      },
      size: {
        default:
          "oe-h-7 oe-gap-1 oe-px-2 oe-text-xs oe-leading-relaxed has-[[data-icon=inline-end]]:oe-pr-1.5 has-[[data-icon=inline-start]]:oe-pl-1.5 [&_svg:not([class*='size-'])]:oe-size-3.5",
        xs: "oe-h-5 oe-gap-1 oe-rounded-sm oe-px-2 oe-text-[0.625rem] has-[[data-icon=inline-end]]:oe-pr-1.5 has-[[data-icon=inline-start]]:oe-pl-1.5 [&_svg:not([class*='size-'])]:oe-size-2.5",
        sm: "oe-h-6 oe-gap-1 oe-px-2 oe-text-xs oe-leading-relaxed has-[[data-icon=inline-end]]:oe-pr-1.5 has-[[data-icon=inline-start]]:oe-pl-1.5 [&_svg:not([class*='size-'])]:oe-size-3",
        lg: "oe-h-8 oe-gap-1 oe-px-2.5 oe-text-xs oe-leading-relaxed has-[[data-icon=inline-end]]:oe-pr-2 has-[[data-icon=inline-start]]:oe-pl-2 [&_svg:not([class*='size-'])]:oe-size-4",
        icon: "oe-size-7 [&_svg:not([class*='size-'])]:oe-size-3.5",
        "icon-xs":
          "oe-size-5 oe-rounded-sm [&_svg:not([class*='size-'])]:oe-size-2.5",
        "icon-sm": "oe-size-6 [&_svg:not([class*='size-'])]:oe-size-3",
        "icon-lg": "oe-size-8 [&_svg:not([class*='size-'])]:oe-size-4",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);
function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
  }) {
  const Comp = asChild ? Slot : "button";
  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}
export { Button, buttonVariants };
