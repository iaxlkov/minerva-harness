"use client"

import * as React from "react"
import { cn } from "../../lib/utils"
import * as TooltipPrimitive from "@radix-ui/react-tooltip"

function TooltipProvider({
  delayDuration = 0,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delayDuration={delayDuration}
      {...props}
    />
  )
}

function Tooltip({
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Root>) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />
}

function TooltipTrigger({
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />
}

function TooltipContent({
  className,
  sideOffset = 0,
  children,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          "oe-z-50 oe-inline-flex oe-w-fit oe-max-w-xs oe-origin-[var(--radix-tooltip-content-transform-origin)] oe-items-center oe-gap-1.5 oe-rounded-md oe-bg-foreground oe-px-3 oe-py-1.5 oe-text-xs oe-text-background has-data-[slot=kbd]:oe-pr-1.5 data-[side=bottom]:oe-slide-in-from-top-2 data-[side=left]:oe-slide-in-from-right-2 data-[side=right]:oe-slide-in-from-left-2 data-[side=top]:oe-slide-in-from-bottom-2 **:data-[slot=kbd]:oe-relative **:data-[slot=kbd]:oe-isolate **:data-[slot=kbd]:oe-z-50 **:data-[slot=kbd]:oe-rounded-sm data-[state=delayed-open]:oe-animate-in data-[state=delayed-open]:oe-fade-in-0 data-[state=delayed-open]:oe-zoom-in-95 data-[state=open]:oe-animate-in data-[state=open]:oe-fade-in-0 data-[state=open]:oe-zoom-in-95 data-[state=closed]:oe-animate-out data-[state=closed]:oe-fade-out-0 data-[state=closed]:oe-zoom-out-95",
          className
        )}
        {...props}
      >
        {children}
        <TooltipPrimitive.Arrow className="oe-z-50 oe-size-2.5 oe-translate-y-[calc(-50%_-_2px)] oe-rotate-45 oe-rounded-[2px] oe-bg-foreground oe-fill-foreground" />
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  )
}

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger }
