import * as React from "react"
import { cn } from "../../lib/utils"

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "oe-flex oe-min-h-16 oe-w-full oe-min-w-0 oe-rounded-md oe-border oe-border-solid oe-border-border oe-bg-input oe-px-2 oe-py-1.5 oe-text-foreground oe-text-xs/relaxed oe-transition-colors oe-outline-none oe-resize-y placeholder:oe-text-muted-foreground focus-visible:oe-border-ring focus-visible:oe-ring-2 focus-visible:oe-ring-ring disabled:oe-cursor-not-allowed disabled:oe-opacity-50",
        className
      )}
      {...props}
    />
  )
}

export { Textarea }
