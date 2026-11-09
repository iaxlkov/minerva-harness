import * as React from "react"
import { cn } from "../../lib/utils"
import * as SeparatorPrimitive from "@radix-ui/react-separator"

function Separator({
  className,
  orientation = "horizontal",
  decorative = true,
  ...props
}: React.ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      data-slot="separator"
      decorative={decorative}
      orientation={orientation}
      className={cn(
        "oe-shrink-0 oe-bg-border data-[orientation=horizontal]:oe-h-px data-[orientation=horizontal]:oe-w-full data-[orientation=vertical]:oe-w-px data-[orientation=vertical]:oe-self-stretch",
        className
      )}
      {...props}
    />
  )
}

export { Separator }
