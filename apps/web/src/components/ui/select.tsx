import { ScrollArea } from './scroll-area';
import * as React from "react";
import * as SelectPrimitive from "@radix-ui/react-select";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "../../lib/utils";

const Select = SelectPrimitive.Root;
const SelectValue = SelectPrimitive.Value;
const SelectGroup = SelectPrimitive.Group;
function SelectLabel(props: React.ComponentProps<typeof SelectPrimitive.Label>) { return <SelectPrimitive.Label className="ds-select-label" {...props}/>; }
function SelectTrigger({ className, children, ...props }: React.ComponentProps<typeof SelectPrimitive.Trigger>) {
  return <SelectPrimitive.Trigger {...(!props.asChild&&{"data-slot":"select-trigger"})} className={cn(!props.asChild&&"ds-select-trigger", className)} {...props}>{props.asChild?children:<>{children}<SelectPrimitive.Icon asChild><ChevronDown size={15}/></SelectPrimitive.Icon></>}</SelectPrimitive.Trigger>;
}
function SelectContent({ children, className, ...props }: React.ComponentProps<typeof SelectPrimitive.Content>) {
  return <SelectPrimitive.Portal><SelectPrimitive.Content data-slot="select-content" position="popper" sideOffset={4} className={cn("oe-studio ds-select-content",className)} {...props}><ScrollArea asChild><SelectPrimitive.Viewport className="ds-select-viewport">{children}</SelectPrimitive.Viewport></ScrollArea></SelectPrimitive.Content></SelectPrimitive.Portal>;
}
function SelectItem({ children, className, ...props }: React.ComponentProps<typeof SelectPrimitive.Item>) {
  return <SelectPrimitive.Item data-slot="select-item" className={cn("ds-select-item",className)} {...props}><SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText><SelectPrimitive.ItemIndicator className="ds-select-check"><Check size={14}/></SelectPrimitive.ItemIndicator></SelectPrimitive.Item>;
}
export { Select, SelectValue, SelectGroup, SelectLabel, SelectTrigger, SelectContent, SelectItem };
