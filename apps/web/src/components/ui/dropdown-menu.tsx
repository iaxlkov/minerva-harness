import * as React from 'react';
import * as Menu from '@radix-ui/react-dropdown-menu';
import {Check, ChevronRight} from 'lucide-react';
import {cn} from '../../lib/utils';
export const DropdownMenu=Menu.Root;
export const DropdownMenuTrigger=Menu.Trigger;
export const DropdownMenuGroup=Menu.Group;
export function DropdownMenuLabel({className,...props}:React.ComponentProps<typeof Menu.Label>){
 return <Menu.Label className={cn('ds-dropdown-label',className)} {...props}/>;
}
export const DropdownMenuRadioGroup=Menu.RadioGroup;
export const DropdownMenuSub=Menu.Sub;
export function DropdownMenuContent({className,...props}:React.ComponentProps<typeof Menu.Content>){
 return <Menu.Portal><Menu.Content data-slot="dropdown-menu-content" className={cn('oe-studio ds-dropdown-menu',className)} sideOffset={4} {...props}/></Menu.Portal>;
}
export function DropdownMenuItem({className,variant='default',...props}:React.ComponentProps<typeof Menu.Item>&{variant?:'default'|'destructive'}){
 return <Menu.Item data-slot="dropdown-menu-item" data-variant={variant} className={cn('ds-dropdown-item',className)} {...props}/>;
}
export function DropdownMenuSeparator({className,...props}:React.ComponentProps<typeof Menu.Separator>){
 return <Menu.Separator className={cn('ds-dropdown-separator',className)} {...props}/>;
}
export function DropdownMenuRadioItem({className,children,...props}:React.ComponentProps<typeof Menu.RadioItem>){
 return <Menu.RadioItem data-slot="dropdown-menu-radio-item" className={cn('ds-dropdown-item ds-dropdown-check',className)} {...props}><span className="ds-dropdown-item-text">{children}</span><Menu.ItemIndicator className="ds-dropdown-indicator"><Check/></Menu.ItemIndicator></Menu.RadioItem>;
}
export function DropdownMenuCheckboxItem({className,...props}:React.ComponentProps<typeof Menu.CheckboxItem>){
 return <Menu.CheckboxItem data-slot="dropdown-menu-checkbox-item" className={cn('ds-dropdown-item',className)} {...props}/>;
}
export function DropdownMenuSubTrigger({className,children,...props}:React.ComponentProps<typeof Menu.SubTrigger>){
 return <Menu.SubTrigger data-slot="dropdown-menu-sub-trigger" className={cn('ds-dropdown-item',className)} {...props}>{children}<ChevronRight className="ds-dropdown-sub-chevron"/></Menu.SubTrigger>;
}
export function DropdownMenuSubContent({className,...props}:React.ComponentProps<typeof Menu.SubContent>){
 return <Menu.Portal><Menu.SubContent data-slot="dropdown-menu-sub-content" className={cn('oe-studio ds-dropdown-menu',className)} sideOffset={6} {...props}/></Menu.Portal>;
}
