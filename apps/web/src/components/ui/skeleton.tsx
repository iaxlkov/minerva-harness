import { cn } from "../../lib/utils"

function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("oe-animate-pulse oe-rounded-md oe-bg-muted", className)}
      {...props}
    />
  )
}

export { Skeleton }
