import React, {useMemo} from 'react';
import {marked} from 'marked';
import DOMPurify from 'dompurify';
import {ScrollArea} from '../../components/ui/scroll-area';
import {resolveLink} from './markdownPath';

/** Rendered markdown; DOMPurify strips scripts and handlers since workspace files are untrusted. */
export function MarkdownPreview({path, text, onOpen}: {path: string; text: string; onOpen: (path: string) => void}) {
  const html = useMemo(() => DOMPurify.sanitize(marked.parse(text, {async: false, gfm: true}) as string), [text]);
  const click = (event: React.MouseEvent<HTMLElement>) => {
    const link = (event.target as Element).closest('a');
    if (!link) return;
    // Links must never navigate the app window away; in-page anchors scroll, relative files open in the editor.
    event.preventDefault();
    const href = link.getAttribute('href') ?? '';
    if (href.startsWith('#')) { event.currentTarget.querySelector(`[id="${CSS.escape(decodeURIComponent(href.slice(1)))}"]`)?.scrollIntoView({block: 'start'}); return; }
    const target = resolveLink(path, href);
    if (target) onOpen(target);
  };
  return <ScrollArea className="markdown-preview"><article className="markdown-body" onClick={click} dangerouslySetInnerHTML={{__html: html}}/></ScrollArea>;
}
