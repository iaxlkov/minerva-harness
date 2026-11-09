import * as React from "react"
import { cn } from "../../lib/utils"
import * as SheetPrimitive from "@radix-ui/react-dialog"

import { Button } from "./button"
import { XIcon } from "lucide-react"

function Sheet({ ...props }: React.ComponentProps<typeof SheetPrimitive.Root>) {
  return <SheetPrimitive.Root data-slot="sheet" {...props} />
}

function SheetTrigger({
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Trigger>) {
  return <SheetPrimitive.Trigger data-slot="sheet-trigger" {...props} />
}

function SheetClose({
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Close>) {
  return <SheetPrimitive.Close data-slot="sheet-close" {...props} />
}

function SheetPortal({
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Portal>) {
  return <SheetPrimitive.Portal data-slot="sheet-portal" {...props} />
}

function SheetOverlay({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Overlay>) {
  return (
    <SheetPrimitive.Overlay
      data-slot="sheet-overlay"
      className={cn(
        "oe-fixed oe-inset-0 oe-z-50 oe-bg-black/80 oe-duration-100 supports-backdrop-filter:oe-backdrop-blur-xs data-[state=open]:oe-animate-in data-[state=open]:oe-fade-in-0 data-[state=closed]:oe-animate-out data-[state=closed]:oe-fade-out-0",
        className
      )}
      {...props}
    />
  )
}

function SheetContent({
  className,
  children,
  side = "right",
  showCloseButton = true,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Content> & {
  side?: "top" | "right" | "bottom" | "left"
  showCloseButton?: boolean
}) {
  return (
    <SheetPortal>
      <SheetOverlay />
      <SheetPrimitive.Content
        data-slot="sheet-content"
        data-side={side}
        className={cn(
          "oe-fixed oe-z-50 oe-flex oe-flex-col oe-bg-popover oe-bg-clip-padding oe-text-xs/relaxed oe-text-popover-foreground oe-shadow-lg oe-transition oe-duration-200 oe-ease-in-out data-[side=bottom]:oe-inset-x-0 data-[side=bottom]:oe-bottom-0 data-[side=bottom]:oe-h-auto data-[side=bottom]:oe-border-t data-[side=left]:oe-inset-y-0 data-[side=left]:oe-left-0 data-[side=left]:oe-h-full data-[side=left]:oe-w-3/4 data-[side=left]:oe-border-r data-[side=right]:oe-inset-y-0 data-[side=right]:oe-right-0 data-[side=right]:oe-h-full data-[side=right]:oe-w-3/4 data-[side=right]:oe-border-l data-[side=top]:oe-inset-x-0 data-[side=top]:oe-top-0 data-[side=top]:oe-h-auto data-[side=top]:oe-border-b data-[side=left]:sm:oe-max-w-sm data-[side=right]:sm:oe-max-w-sm data-[state=open]:oe-animate-in data-[state=open]:oe-fade-in-0 data-[side=bottom]:data-[state=open]:oe-slide-in-from-bottom-10 data-[side=left]:data-[state=open]:oe-slide-in-from-left-10 data-[side=right]:data-[state=open]:oe-slide-in-from-right-10 data-[side=top]:data-[state=open]:oe-slide-in-from-top-10 data-[state=closed]:oe-animate-out data-[state=closed]:oe-fade-out-0 data-[side=bottom]:data-[state=closed]:oe-slide-out-to-bottom-10 data-[side=left]:data-[state=closed]:oe-slide-out-to-left-10 data-[side=right]:data-[state=closed]:oe-slide-out-to-right-10 data-[side=top]:data-[state=closed]:oe-slide-out-to-top-10",
          className
        )}
        {...props}
      >
        {children}
        {showCloseButton && (
          <SheetPrimitive.Close data-slot="sheet-close" asChild>
            <Button
              variant="ghost"
              className="oe-absolute oe-top-4 oe-right-4"
              size="icon-sm"
            >
              <XIcon
              />
              <span className="oe-sr-only">Закрыть боковую панель</span>
            </Button>
          </SheetPrimitive.Close>
        )}
      </SheetPrimitive.Content>
    </SheetPortal>
  )
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-header"
      className={cn("oe-flex oe-flex-col oe-gap-1.5 oe-p-6", className)}
      {...props}
    />
  )
}

function SheetFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-footer"
      className={cn("oe-mt-auto oe-flex oe-flex-col oe-gap-2 oe-p-6", className)}
      {...props}
    />
  )
}

function SheetTitle({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Title>) {
  return (
    <SheetPrimitive.Title
      data-slot="sheet-title"
      className={cn(
        "oe- oe-text-sm oe-font-medium oe-text-foreground",
        className
      )}
      {...props}
    />
  )
}

function SheetDescription({
  className,
  ...props
}: React.ComponentProps<typeof SheetPrimitive.Description>) {
  return (
    <SheetPrimitive.Description
      data-slot="sheet-description"
      className={cn("oe-text-xs/relaxed oe-text-muted-foreground", className)}
      {...props}
    />
  )
}

export {
  Sheet,
  SheetTrigger,
  SheetClose,
  SheetContent,
  SheetHeader,
  SheetFooter,
  SheetTitle,
  SheetDescription,
}
