import * as React from "react"
import * as SwitchPrimitive from "@radix-ui/react-switch"
import { cn } from "../../lib/utils"

function Switch({
  className,
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        // No Tailwind preflight/base here: reset the button and avoid --tw-* driven ring/translate utilities.
        "oe-peer oe-inline-flex oe-h-[1.15rem] oe-w-8 oe-shrink-0 oe-cursor-pointer oe-items-center oe-rounded-full oe-border oe-border-solid oe-border-transparent oe-p-0 oe-transition-all oe-outline-none focus-visible:oe-outline focus-visible:oe-outline-2 focus-visible:oe-outline-offset-2 focus-visible:oe-outline-ring disabled:oe-cursor-not-allowed disabled:oe-opacity-50 data-[state=checked]:oe-bg-primary data-[state=unchecked]:oe-bg-border",
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className="oe-pointer-events-none oe-block oe-size-4 oe-rounded-full oe-ring-0 oe-transition-transform data-[state=checked]:[transform:translateX(calc(100%_-_2px))] data-[state=checked]:oe-bg-primary-foreground data-[state=unchecked]:[transform:translateX(0)] data-[state=unchecked]:oe-bg-foreground"
      />
    </SwitchPrimitive.Root>
  )
}

export { Switch }
