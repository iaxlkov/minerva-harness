import {useEffect,useRef,type ComponentProps,type RefObject} from 'react';
import {cn} from '../../lib/utils';

// assistant-ui Elements ComposerMenu layout, adapted to our full-width composer.
// https://github.com/assistant-ui/assistant-ui/blob/main/packages/ui/src/components/react/assistant-ui/elements/composer.tsx
export function ComposerMenu({open,onClose,triggerRef,className,children,...props}:ComponentProps<'div'>&{
 open:boolean;onClose:()=>void;triggerRef:RefObject<HTMLButtonElement|null>;
}){
 const menu=useRef<HTMLDivElement>(null);
 useEffect(()=>{
  if(!open)return;
  menu.current?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus();
  const dismiss=(event:PointerEvent)=>{
   if(event.target instanceof Node&&!menu.current?.contains(event.target)&&!triggerRef.current?.contains(event.target))onClose();
  };
  document.addEventListener('pointerdown',dismiss);
  return ()=>document.removeEventListener('pointerdown',dismiss);
 },[open,onClose,triggerRef]);
 if(!open)return null;
 return <div {...props} ref={menu} role="menu" data-slot="composer-menu" className={cn('aui-composer-menu',className)} onKeyDown={event=>{
  if(event.key==='Escape'){event.preventDefault();event.stopPropagation();onClose();return;}
  if(event.key==='Tab'){onClose();return;}
  if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
  event.preventDefault();
  const items=Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')??[]);
  if(!items.length)return;
  const current=items.indexOf(document.activeElement as HTMLButtonElement);
  const index=event.key==='Home'?0:event.key==='End'?items.length-1:(current+(event.key==='ArrowDown'?1:-1)+items.length)%items.length;
  items[index]?.focus();
 }} onClick={event=>{
  if(!event.defaultPrevented&&event.target instanceof Element&&event.target.closest('[role="menuitem"]'))onClose();
 }}>{children}</div>;
}

export function ComposerMenuItem({className,...props}:ComponentProps<'button'>){
 return <button {...props} type="button" role="menuitem" tabIndex={-1} data-slot="composer-menu-item" className={cn('aui-composer-menu-item',className)}/>;
}
