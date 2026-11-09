"use client"

import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "../../lib/utils"
import { Slot as RadixSlot } from "@radix-ui/react-slot"
const Slot = {Root: RadixSlot}

import { useIsMobile } from "../../hooks/use-mobile"
import { Button } from "./button"
import { Input } from "./input"
import { Separator } from "./separator"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "./sheet"
import { Skeleton } from "./skeleton"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "./tooltip"
import { PanelLeftIcon } from "lucide-react"

const SIDEBAR_COOKIE_NAME = "sidebar_state"
const SIDEBAR_COOKIE_MAX_AGE = 60 * 60 * 24 * 7
const SIDEBAR_WIDTH = "16rem"
const SIDEBAR_WIDTH_MOBILE = "18rem"
const SIDEBAR_WIDTH_ICON = "3rem"
const SIDEBAR_KEYBOARD_SHORTCUT = "b"

type SidebarContextProps = {
  state: "expanded" | "collapsed"
  open: boolean
  setOpen: (open: boolean) => void
  openMobile: boolean
  setOpenMobile: (open: boolean) => void
  isMobile: boolean
  toggleSidebar: () => void
}

const SidebarContext = React.createContext<SidebarContextProps | null>(null)

function useSidebar() {
  const context = React.useContext(SidebarContext)
  if (!context) {
    throw new Error("useSidebar must be used within a SidebarProvider.")
  }

  return context
}

function SidebarProvider({
  defaultOpen = true,
  open: openProp,
  onOpenChange: setOpenProp,
  className,
  style,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  defaultOpen?: boolean
  open?: boolean
  onOpenChange?: (open: boolean) => void
}) {
  const isMobile = useIsMobile()
  const [openMobile, setOpenMobile] = React.useState(false)

  // This is the internal state of the sidebar.
  // We use openProp and setOpenProp for control from outside the component.
  const [_open, _setOpen] = React.useState(defaultOpen)
  const open = openProp ?? _open
  const setOpen = React.useCallback(
    (value: boolean | ((value: boolean) => boolean)) => {
      const openState = typeof value === "function" ? value(open) : value
      if (setOpenProp) {
        setOpenProp(openState)
      } else {
        _setOpen(openState)
      }

      // This sets the cookie to keep the sidebar state.
      document.cookie = `${SIDEBAR_COOKIE_NAME}=${openState}; path=/; max-age=${SIDEBAR_COOKIE_MAX_AGE}`
    },
    [setOpenProp, open]
  )

  // Helper to toggle the sidebar.
  const toggleSidebar = React.useCallback(() => {
    return isMobile ? setOpenMobile((open) => !open) : setOpen((open) => !open)
  }, [isMobile, setOpen, setOpenMobile])

  // Adds a keyboard shortcut to toggle the sidebar.
  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        event.key === SIDEBAR_KEYBOARD_SHORTCUT &&
        (event.metaKey || event.ctrlKey)
      ) {
        event.preventDefault()
        toggleSidebar()
      }
    }

    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [toggleSidebar])

  // We add a state so that we can do data-state="expanded" or "collapsed".
  // This makes it easier to style the sidebar with Tailwind classes.
  const state = open ? "expanded" : "collapsed"

  const contextValue = React.useMemo<SidebarContextProps>(
    () => ({
      state,
      open,
      setOpen,
      isMobile,
      openMobile,
      setOpenMobile,
      toggleSidebar,
    }),
    [state, open, setOpen, isMobile, openMobile, setOpenMobile, toggleSidebar]
  )

  return (
    <SidebarContext.Provider value={contextValue}>
      <div
        data-slot="sidebar-wrapper"
        style={
          {
            "--sidebar-width": SIDEBAR_WIDTH,
            "--sidebar-width-icon": SIDEBAR_WIDTH_ICON,
            ...style,
          } as React.CSSProperties
        }
        className={cn(
          "oe-group/sidebar-wrapper oe-flex oe-min-h-svh oe-w-full has-data-[variant=inset]:oe-bg-sidebar",
          className
        )}
        {...props}
      >
        {children}
      </div>
    </SidebarContext.Provider>
  )
}

function Sidebar({
  side = "left",
  variant = "sidebar",
  collapsible = "offcanvas",
  className,
  children,
  dir,
  ...props
}: React.ComponentProps<"div"> & {
  side?: "left" | "right"
  variant?: "sidebar" | "floating" | "inset"
  collapsible?: "offcanvas" | "icon" | "none"
}) {
  const { isMobile, state, openMobile, setOpenMobile } = useSidebar()

  if (collapsible === "none") {
    return (
      <div
        data-slot="sidebar"
        className={cn(
          "oe-flex oe-h-full oe-w-[var(--sidebar-width)] oe-flex-col oe-bg-sidebar oe-text-sidebar-foreground",
          className
        )}
        {...props}
      >
        {children}
      </div>
    )
  }

  if (isMobile) {
    return (
      <Sheet open={openMobile} onOpenChange={setOpenMobile} {...props}>
        <SheetContent
          dir={dir}
          data-sidebar="sidebar"
          data-slot="sidebar"
          data-mobile="true"
          className="oe-w-[var(--sidebar-width)] oe-bg-sidebar oe-p-0 oe-text-sidebar-foreground [&>button]:oe-hidden"
          style={
            {
              "--sidebar-width": SIDEBAR_WIDTH_MOBILE,
            } as React.CSSProperties
          }
          side={side}
        >
          <SheetHeader className="oe-sr-only">
            <SheetTitle>Sidebar</SheetTitle>
            <SheetDescription>Displays the mobile sidebar.</SheetDescription>
          </SheetHeader>
          <div className="oe-flex oe-h-full oe-w-full oe-flex-col">{children}</div>
        </SheetContent>
      </Sheet>
    )
  }

  return (
    <div
      className="oe-group oe-peer oe-hidden oe-text-sidebar-foreground md:oe-block"
      data-state={state}
      data-collapsible={state === "collapsed" ? collapsible : ""}
      data-variant={variant}
      data-side={side}
      data-slot="sidebar"
    >
      {/* This is what handles the sidebar gap on desktop */}
      <div
        data-slot="sidebar-gap"
        className={cn(
          "oe-relative oe-w-[var(--sidebar-width)] oe-bg-transparent oe-transition-[width] oe-duration-200 oe-ease-linear",
          "group-data-[collapsible=offcanvas]:oe-w-0",
          "group-data-[side=right]:oe-rotate-180",
          variant === "floating" || variant === "inset"
            ? "group-data-[collapsible=icon]:oe-w-[calc(var(--sidebar-width-icon)+(1rem))]"
            : "group-data-[collapsible=icon]:oe-w-[var(--sidebar-width-icon)]"
        )}
      />
      <div
        data-slot="sidebar-container"
        data-side={side}
        className={cn(
          "oe-fixed oe-inset-y-0 oe-z-10 oe-hidden oe-h-svh oe-w-[var(--sidebar-width)] oe-transition-[left,right,width] oe-duration-200 oe-ease-linear data-[side=left]:oe-left-0 data-[side=left]:group-data-[collapsible=offcanvas]:oe-left-[calc(var(--sidebar-width)*-1)] data-[side=right]:oe-right-0 data-[side=right]:group-data-[collapsible=offcanvas]:oe-right-[calc(var(--sidebar-width)*-1)] md:oe-flex",
          // Adjust the padding for floating and inset variants.
          variant === "floating" || variant === "inset"
            ? "oe-p-2 group-data-[collapsible=icon]:oe-w-[calc(var(--sidebar-width-icon)+(1rem)+2px)]"
            : "group-data-[collapsible=icon]:oe-w-[var(--sidebar-width-icon)] group-data-[side=left]:oe-border-r group-data-[side=right]:oe-border-l",
          className
        )}
        {...props}
      >
        <div
          data-sidebar="sidebar"
          data-slot="sidebar-inner"
          className="oe-flex oe-size-full oe-flex-col oe-bg-sidebar group-data-[variant=floating]:oe-rounded-lg group-data-[variant=floating]:oe-shadow-sm group-data-[variant=floating]:oe-ring-1 group-data-[variant=floating]:oe-ring-sidebar-border"
        >
          {children}
        </div>
      </div>
    </div>
  )
}

function SidebarTrigger({
  className,
  onClick,
  ...props
}: React.ComponentProps<typeof Button>) {
  const { toggleSidebar } = useSidebar()

  return (
    <Button
      data-sidebar="trigger"
      data-slot="sidebar-trigger"
      variant="ghost"
      size="icon-sm"
      className={cn(className)}
      onClick={(event) => {
        onClick?.(event)
        toggleSidebar()
      }}
      {...props}
    >
      <PanelLeftIcon />
      <span className="oe-sr-only">Toggle Sidebar</span>
    </Button>
  )
}

function SidebarRail({ className, ...props }: React.ComponentProps<"button">) {
  const { toggleSidebar } = useSidebar()

  return (
    <button
      data-sidebar="rail"
      data-slot="sidebar-rail"
      aria-label="Toggle Sidebar"
      tabIndex={-1}
      onClick={toggleSidebar}
      title="Toggle Sidebar"
      className={cn(
        "oe-absolute oe-inset-y-0 oe-z-20 oe-hidden oe-w-4 oe-transition-all oe-ease-linear group-data-[side=left]:oe--right-4 group-data-[side=right]:oe-left-0 after:oe-absolute after:oe-inset-y-0 after:oe-start-1/2 after:oe-w-[2px] hover:after:oe-bg-sidebar-border sm:oe-flex ltr:oe--translate-x-1/2 rtl:oe--translate-x-1/2",
        "in-data-[side=left]:oe-cursor-w-resize in-data-[side=right]:oe-cursor-e-resize",
        "[[data-side=left][data-state=collapsed]_&]:oe-cursor-e-resize [[data-side=right][data-state=collapsed]_&]:oe-cursor-w-resize",
        "group-data-[collapsible=offcanvas]:oe-translate-x-0 group-data-[collapsible=offcanvas]:after:oe-left-full hover:group-data-[collapsible=offcanvas]:oe-bg-sidebar",
        "[[data-side=left][data-collapsible=offcanvas]_&]:oe--right-2",
        "[[data-side=right][data-collapsible=offcanvas]_&]:oe--left-2",
        className
      )}
      {...props}
    />
  )
}

function SidebarInset({ className, ...props }: React.ComponentProps<"main">) {
  return (
    <main
      data-slot="sidebar-inset"
      className={cn(
        "oe-relative oe-flex oe-w-full oe-flex-1 oe-flex-col oe-bg-background md:peer-data-[variant=inset]:oe-m-2 md:peer-data-[variant=inset]:oe-ml-0 md:peer-data-[variant=inset]:oe-rounded-xl md:peer-data-[variant=inset]:oe-shadow-sm md:peer-data-[variant=inset]:peer-data-[state=collapsed]:oe-ml-2",
        className
      )}
      {...props}
    />
  )
}

function SidebarInput({
  className,
  ...props
}: React.ComponentProps<typeof Input>) {
  return (
    <Input
      data-slot="sidebar-input"
      data-sidebar="input"
      className={cn(
        "oe-h-8 oe-w-full oe-border-input oe-bg-muted/20 dark:oe-bg-muted/30",
        className
      )}
      {...props}
    />
  )
}

function SidebarHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-header"
      data-sidebar="header"
      className={cn("oe-flex oe-flex-col oe-gap-2 oe-p-2", className)}
      {...props}
    />
  )
}

function SidebarFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-footer"
      data-sidebar="footer"
      className={cn("oe-flex oe-flex-col oe-gap-2 oe-p-2", className)}
      {...props}
    />
  )
}

function SidebarSeparator({
  className,
  ...props
}: React.ComponentProps<typeof Separator>) {
  return (
    <Separator
      data-slot="sidebar-separator"
      data-sidebar="separator"
      className={cn("oe-mx-2 oe-w-auto oe-bg-sidebar-border", className)}
      {...props}
    />
  )
}

function SidebarContent({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-content"
      data-sidebar="content"
      className={cn(
        "oe-no-scrollbar oe-flex oe-min-h-0 oe-flex-1 oe-flex-col oe-gap-0 oe-overflow-auto group-data-[collapsible=icon]:oe-overflow-hidden",
        className
      )}
      {...props}
    />
  )
}

function SidebarGroup({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-group"
      data-sidebar="group"
      className={cn(
        "oe-relative oe-flex oe-w-full oe-min-w-0 oe-flex-col oe-px-2 oe-py-1",
        className
      )}
      {...props}
    />
  )
}

function SidebarGroupLabel({
  className,
  asChild = false,
  ...props
}: React.ComponentProps<"div"> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : "div"

  return (
    <Comp
      data-slot="sidebar-group-label"
      data-sidebar="group-label"
      className={cn(
        "oe-flex oe-h-8 oe-shrink-0 oe-items-center oe-rounded-md oe-px-2 oe-text-xs oe-text-sidebar-foreground/70 oe-ring-sidebar-ring oe-outline-none oe-transition-[margin,opacity] oe-duration-200 oe-ease-linear group-data-[collapsible=icon]:oe--mt-8 group-data-[collapsible=icon]:oe-opacity-0 focus-visible:oe-ring-2 [&>svg]:oe-size-4 [&>svg]:oe-shrink-0",
        className
      )}
      {...props}
    />
  )
}

function SidebarGroupAction({
  className,
  asChild = false,
  ...props
}: React.ComponentProps<"button"> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="sidebar-group-action"
      data-sidebar="group-action"
      className={cn(
        "oe-absolute oe-top-3.5 oe-right-3 oe-flex oe-aspect-square oe-w-5 oe-items-center oe-justify-center oe-rounded-md oe-p-0 oe-text-sidebar-foreground oe-ring-sidebar-ring oe-outline-none oe-transition-transform group-data-[collapsible=icon]:oe-hidden after:oe-absolute after:oe--inset-2 hover:oe-bg-sidebar-accent hover:oe-text-sidebar-accent-foreground focus-visible:oe-ring-2 md:after:oe-hidden [&>svg]:oe-size-4 [&>svg]:oe-shrink-0",
        className
      )}
      {...props}
    />
  )
}

function SidebarGroupContent({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-group-content"
      data-sidebar="group-content"
      className={cn("oe-w-full oe-text-xs", className)}
      {...props}
    />
  )
}

function SidebarMenu({ className, ...props }: React.ComponentProps<"ul">) {
  return (
    <ul
      data-slot="sidebar-menu"
      data-sidebar="menu"
      className={cn("oe-flex oe-w-full oe-min-w-0 oe-flex-col oe-gap-px", className)}
      {...props}
    />
  )
}

function SidebarMenuItem({ className, ...props }: React.ComponentProps<"li">) {
  return (
    <li
      data-slot="sidebar-menu-item"
      data-sidebar="menu-item"
      className={cn("oe-group/menu-item oe-relative", className)}
      {...props}
    />
  )
}

const sidebarMenuButtonVariants = cva(
  "oe-peer/menu-button oe-group/menu-button oe-flex oe-w-full oe-items-center oe-gap-2 oe-overflow-hidden oe-rounded-[calc(var(--radius-sm)+2px)] oe-p-2 oe-text-left oe-text-xs oe-ring-sidebar-ring oe-outline-none oe-transition-[width,height,padding] group-has-data-[sidebar=menu-action]/menu-item:oe-pr-8 group-data-[collapsible=icon]:oe-size-8! group-data-[collapsible=icon]:oe-p-2! hover:oe-bg-sidebar-accent hover:oe-text-sidebar-accent-foreground focus-visible:oe-ring-2 active:oe-bg-sidebar-accent active:oe-text-sidebar-accent-foreground disabled:oe-pointer-events-none disabled:oe-opacity-50 aria-disabled:oe-pointer-events-none aria-disabled:oe-opacity-50 data-[state=open]:hover:oe-bg-sidebar-accent data-[state=open]:hover:oe-text-sidebar-accent-foreground data-[active=true]:oe-bg-sidebar-accent data-[active=true]:oe-font-medium data-[active=true]:oe-text-sidebar-accent-foreground [&_svg]:oe-size-4 [&_svg]:oe-shrink-0 [&>span:last-child]:oe-truncate",
  {
    variants: {
      variant: {
        default: "hover:oe-bg-sidebar-accent hover:oe-text-sidebar-accent-foreground",
        outline:
          "oe-bg-background oe-shadow-[0_0_0_1px_var(--sidebar-border)] hover:oe-bg-sidebar-accent hover:oe-text-sidebar-accent-foreground hover:oe-shadow-[0_0_0_1px_var(--sidebar-accent)]",
      },
      size: {
        default: "oe-h-8 oe-text-xs",
        sm: "oe-h-7 oe-text-xs",
        lg: "oe-h-12 oe-text-xs group-data-[collapsible=icon]:oe-p-0!",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function SidebarMenuButton({
  asChild = false,
  isActive = false,
  variant = "default",
  size = "default",
  tooltip,
  className,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean
  isActive?: boolean
  tooltip?: string | React.ComponentProps<typeof TooltipContent>
} & VariantProps<typeof sidebarMenuButtonVariants>) {
  const Comp = asChild ? Slot.Root : "button"
  const { isMobile, state } = useSidebar()

  const button = (
    <Comp
      data-slot="sidebar-menu-button"
      data-sidebar="menu-button"
      data-size={size}
      data-active={isActive}
      className={cn(sidebarMenuButtonVariants({ variant, size }), className)}
      {...props}
    />
  )

  if (!tooltip) {
    return button
  }

  if (typeof tooltip === "string") {
    tooltip = {
      children: tooltip,
    }
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent
        side="right"
        align="center"
        hidden={state !== "collapsed" || isMobile}
        {...tooltip}
      />
    </Tooltip>
  )
}

function SidebarMenuAction({
  className,
  asChild = false,
  showOnHover = false,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean
  showOnHover?: boolean
}) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="sidebar-menu-action"
      data-sidebar="menu-action"
      className={cn(
        "oe-absolute oe-top-1.5 oe-right-1 oe-flex oe-aspect-square oe-w-5 oe-items-center oe-justify-center oe-rounded-[calc(var(--radius-sm)-2px)] oe-p-0 oe-text-sidebar-foreground oe-ring-sidebar-ring oe-outline-none oe-transition-transform group-data-[collapsible=icon]:oe-hidden peer-hover/menu-button:oe-text-sidebar-accent-foreground peer-data-[size=default]/menu-button:oe-top-1.5 peer-data-[size=lg]/menu-button:oe-top-2.5 peer-data-[size=sm]/menu-button:oe-top-1 after:oe-absolute after:oe--inset-2 hover:oe-bg-sidebar-accent hover:oe-text-sidebar-accent-foreground focus-visible:oe-ring-2 md:after:oe-hidden [&>svg]:oe-size-4 [&>svg]:oe-shrink-0",
        showOnHover &&
          "group-focus-within/menu-item:oe-opacity-100 group-hover/menu-item:oe-opacity-100 peer-data-active/menu-button:oe-text-sidebar-accent-foreground aria-expanded:oe-opacity-100 md:oe-opacity-0",
        className
      )}
      {...props}
    />
  )
}

function SidebarMenuBadge({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-menu-badge"
      data-sidebar="menu-badge"
      className={cn(
        "oe-pointer-events-none oe-absolute oe-right-1 oe-flex oe-h-5 oe-min-w-5 oe-items-center oe-justify-center oe-rounded-[calc(var(--radius-sm)-2px)] oe-px-1 oe-text-xs oe-font-medium oe-text-sidebar-foreground oe-tabular-nums oe-select-none group-data-[collapsible=icon]:oe-hidden peer-hover/menu-button:oe-text-sidebar-accent-foreground peer-data-[size=default]/menu-button:oe-top-1.5 peer-data-[size=lg]/menu-button:oe-top-2.5 peer-data-[size=sm]/menu-button:oe-top-1 peer-data-active/menu-button:oe-text-sidebar-accent-foreground",
        className
      )}
      {...props}
    />
  )
}

function SidebarMenuSkeleton({
  className,
  showIcon = false,
  ...props
}: React.ComponentProps<"div"> & {
  showIcon?: boolean
}) {
  // Random width between 50 to 90%.
  const [width] = React.useState(() => {
    return `${Math.floor(Math.random() * 40) + 50}%`
  })

  return (
    <div
      data-slot="sidebar-menu-skeleton"
      data-sidebar="menu-skeleton"
      className={cn("oe-flex oe-h-8 oe-items-center oe-gap-2 oe-rounded-md oe-px-2", className)}
      {...props}
    >
      {showIcon && (
        <Skeleton
          className="oe-size-4 oe-rounded-md"
          data-sidebar="menu-skeleton-icon"
        />
      )}
      <Skeleton
        className="oe-h-4 oe-max-w-(--skeleton-width) oe-flex-1"
        data-sidebar="menu-skeleton-text"
        style={
          {
            "--skeleton-width": width,
          } as React.CSSProperties
        }
      />
    </div>
  )
}

function SidebarMenuSub({ className, ...props }: React.ComponentProps<"ul">) {
  return (
    <ul
      data-slot="sidebar-menu-sub"
      data-sidebar="menu-sub"
      className={cn(
        "oe-mx-3.5 oe-flex oe-min-w-0 oe-translate-x-px oe-flex-col oe-gap-1 oe-border-l oe-border-sidebar-border oe-px-2.5 oe-py-0.5 group-data-[collapsible=icon]:oe-hidden",
        className
      )}
      {...props}
    />
  )
}

function SidebarMenuSubItem({
  className,
  ...props
}: React.ComponentProps<"li">) {
  return (
    <li
      data-slot="sidebar-menu-sub-item"
      data-sidebar="menu-sub-item"
      className={cn("oe-group/menu-sub-item oe-relative", className)}
      {...props}
    />
  )
}

function SidebarMenuSubButton({
  asChild = false,
  size = "md",
  isActive = false,
  className,
  ...props
}: React.ComponentProps<"a"> & {
  asChild?: boolean
  size?: "sm" | "md"
  isActive?: boolean
}) {
  const Comp = asChild ? Slot.Root : "a"

  return (
    <Comp
      data-slot="sidebar-menu-sub-button"
      data-sidebar="menu-sub-button"
      data-size={size}
      data-active={isActive}
      className={cn(
        "oe-flex oe-h-7 oe-min-w-0 oe--translate-x-px oe-items-center oe-gap-2 oe-overflow-hidden oe-rounded-md oe-px-2 oe-text-sidebar-foreground oe-ring-sidebar-ring oe-outline-none group-data-[collapsible=icon]:oe-hidden hover:oe-bg-sidebar-accent hover:oe-text-sidebar-accent-foreground focus-visible:oe-ring-2 active:oe-bg-sidebar-accent active:oe-text-sidebar-accent-foreground disabled:oe-pointer-events-none disabled:oe-opacity-50 aria-disabled:oe-pointer-events-none aria-disabled:oe-opacity-50 data-[size=md]:oe-text-xs data-[size=sm]:oe-text-xs data-[active=true]:oe-bg-sidebar-accent data-[active=true]:oe-text-sidebar-accent-foreground [&>span:last-child]:oe-truncate [&>svg]:oe-size-4 [&>svg]:oe-shrink-0 [&>svg]:oe-text-sidebar-accent-foreground",
        className
      )}
      {...props}
    />
  )
}

export {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInput,
  SidebarInset,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarProvider,
  SidebarRail,
  SidebarSeparator,
  SidebarTrigger,
  useSidebar,
}
