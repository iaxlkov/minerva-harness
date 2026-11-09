import { cn } from "../../lib/utils"
import * as ResizablePrimitive from "react-resizable-panels"

function ResizablePanelGroup({
  className,
  ...props
}: ResizablePrimitive.GroupProps) {
  return (
    <ResizablePrimitive.Group
      data-slot="resizable-panel-group"
      className={cn(
        "oe-flex oe-h-full oe-w-full aria-[orientation=vertical]:oe-flex-col",
        className
      )}
      {...props}
    />
  )
}

function ResizablePanel({ ...props }: ResizablePrimitive.PanelProps) {
  return <ResizablePrimitive.Panel data-slot="resizable-panel" {...props} />
}

function ResizableHandle({
  withHandle,
  className,
  ...props
}: ResizablePrimitive.SeparatorProps & {
  withHandle?: boolean
}) {
  return (
    <ResizablePrimitive.Separator
      data-slot="resizable-handle"
      className={cn(
        "oe-relative oe-flex oe-w-px oe-items-center oe-justify-center oe-bg-border oe-ring-offset-background after:oe-absolute after:oe-inset-y-0 after:oe-left-1/2 after:oe-w-1 after:oe--translate-x-1/2 focus-visible:oe-ring-1 focus-visible:oe-ring-ring focus-visible:oe-outline-none aria-[orientation=horizontal]:oe-h-px aria-[orientation=horizontal]:oe-w-full aria-[orientation=horizontal]:after:oe-left-0 aria-[orientation=horizontal]:after:oe-h-1 aria-[orientation=horizontal]:after:oe-w-full aria-[orientation=horizontal]:after:oe-translate-x-0 aria-[orientation=horizontal]:after:oe--translate-y-1/2 [&[aria-orientation=horizontal]>div]:oe-rotate-90",
        className
      )}
      {...props}
    >
      {withHandle && (
        <div className="oe-z-10 oe-flex oe-h-6 oe-w-1 oe-shrink-0 oe-rounded-lg oe-bg-border" />
      )}
    </ResizablePrimitive.Separator>
  )
}

export { ResizableHandle, ResizablePanel, ResizablePanelGroup }
