import * as React from "react"
import * as SwitchPrimitive from "@radix-ui/react-switch"
import { cn } from "../../lib/utils"

const switchRoot="oe-peer oe-inline-flex oe-h-[1.15rem] oe-w-8 oe-shrink-0 oe-cursor-pointer oe-items-center oe-rounded-full oe-border oe-border-solid oe-border-transparent oe-p-0 oe-transition-all oe-outline-none focus-visible:oe-outline focus-visible:oe-outline-2 focus-visible:oe-outline-offset-2 focus-visible:oe-outline-ring disabled:oe-cursor-not-allowed disabled:oe-opacity-50 data-[state=checked]:oe-bg-primary data-[state=unchecked]:oe-bg-border"
const switchThumb="oe-pointer-events-none oe-block oe-size-4 oe-rounded-full oe-ring-0 oe-transition-transform data-[state=checked]:[transform:translateX(calc(100%_-_2px))] data-[state=checked]:oe-bg-primary-foreground data-[state=unchecked]:[transform:translateX(0)] data-[state=unchecked]:oe-bg-foreground"

function Switch({
  className,
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        // No Tailwind preflight/base here: reset the button and avoid --tw-* driven ring/translate utilities.
        switchRoot,
        className
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={switchThumb}
      />
    </SwitchPrimitive.Root>
  )
}

/** The switch's look without its button, for a menu item that is itself the control. */
function SwitchIndicator({ checked }: { checked: boolean }) {
  const state = checked ? "checked" : "unchecked"
  return <span aria-hidden="true" data-slot="switch" data-state={state} className={cn(switchRoot, "oe-pointer-events-none")}><span data-state={state} className={switchThumb}/></span>
}

export { Switch, SwitchIndicator }
