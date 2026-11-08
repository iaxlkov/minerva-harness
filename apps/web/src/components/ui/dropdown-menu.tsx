import * as React from 'react';
import * as Menu from '@radix-ui/react-dropdown-menu';
import {cn} from '../../lib/utils';
export const DropdownMenu=Menu.Root;
export const DropdownMenuTrigger=Menu.Trigger;
export const DropdownMenuGroup=Menu.Group;
export const DropdownMenuLabel=Menu.Label;
export function DropdownMenuContent({className,...props}:React.ComponentProps<typeof Menu.Content>){
 return <Menu.Portal><Menu.Content data-slot="dropdown-menu-content" className={cn('oe-studio ds-dropdown-menu',className)} sideOffset={4} {...props}/></Menu.Portal>;
}
export function DropdownMenuItem({className,variant='default',...props}:React.ComponentProps<typeof Menu.Item>&{variant?:'default'|'destructive'}){
 return <Menu.Item data-slot="dropdown-menu-item" data-variant={variant} className={cn('ds-dropdown-item',className)} {...props}/>;
}
export function DropdownMenuSeparator({className,...props}:React.ComponentProps<typeof Menu.Separator>){
 return <Menu.Separator className={cn('ds-dropdown-separator',className)} {...props}/>;
}
