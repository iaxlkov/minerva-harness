import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import { cn } from '../../lib/utils';

// Keep the scrolling element itself so chat autoscroll, refs and native controls still work.
export function ScrollArea({ asChild = false, className, ...props }: React.ComponentProps<'div'> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : 'div';
  return <Comp data-slot="scroll-area" className={cn('ds-scroll-area', className)} {...props}/>;
}
