import * as React from "react"
import { cn } from "../../lib/utils"

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "oe-h-7 oe-w-full oe-min-w-0 oe-rounded-md oe-border oe-border-input oe-bg-input/20 oe-px-2 oe-py-0.5 oe-text-sm oe-transition-colors oe-outline-none file:oe-inline-flex file:oe-h-6 file:oe-border-0 file:oe-bg-transparent file:oe-text-xs/relaxed file:oe-font-medium file:oe-text-foreground placeholder:oe-text-muted-foreground focus-visible:oe-border-ring focus-visible:oe-ring-2 focus-visible:oe-ring-ring/30 disabled:oe-pointer-events-none disabled:oe-cursor-not-allowed disabled:oe-opacity-50 aria-invalid:oe-border-destructive aria-invalid:oe-ring-2 aria-invalid:oe-ring-destructive/20 md:oe-text-xs/relaxed dark:oe-bg-input/30 dark:aria-invalid:oe-border-destructive/50 dark:aria-invalid:oe-ring-destructive/40",
        className
      )}
      {...props}
    />
  )
}

export { Input }
