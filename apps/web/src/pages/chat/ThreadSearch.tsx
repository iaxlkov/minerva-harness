import React, {useEffect, useMemo, useRef, useState} from 'react';
import {useAuiState} from '@assistant-ui/react';
import {ConversationSearch} from '../../components/assistant-ui/elements/conversation-search';
import {findHits} from './search';

type Highlights = {set(name: string, value: unknown): void; delete(name: string): void};
// CSS Custom Highlight API: marks matches without touching React-owned DOM; absent in older webviews.
const highlights = (globalThis as {CSS?: {highlights?: Highlights}}).CSS?.highlights;
const HighlightCtor = (globalThis as {Highlight?: new (...ranges: Range[]) => unknown}).Highlight;

function rangesIn(element: Element, needle: string) {
  const ranges: Range[] = [];
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = (node.textContent ?? '').toLocaleLowerCase();
    for (let at = text.indexOf(needle); at !== -1; at = text.indexOf(needle, at + needle.length)) {
      const range = document.createRange();
      range.setStart(node, at); range.setEnd(node, at + needle.length);
      ranges.push(range);
    }
  }
  return ranges;
}

/** Ctrl/⌘+F find bar over the open thread, built on the assistant-ui Elements conversation search. */
export function ThreadSearch() {
  const root = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const messages = useAuiState(state => state.thread.messages);
  const searchable = useMemo(() => messages.map(message => ({
    id: message.id,
    text: message.content.map(part => part.type === 'text' || part.type === 'reasoning' ? part.text : '').join('\n'),
  })), [messages]);
  const hits = useMemo(() => findHits(searchable, query), [searchable, query]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'f' || event.altKey) return;
      // Monaco and hidden threads (other sections) keep their own find.
      if ((event.target as Element | null)?.closest?.('.monaco-editor') || !root.current?.offsetParent) return;
      event.preventDefault();
      setOpen(true);
      requestAnimationFrame(() => { input.current?.focus(); input.current?.select(); });
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
  useEffect(() => setActive(0), [query]);

  const index = hits.length ? ((active % hits.length) + hits.length) % hits.length : -1;
  const hit = index >= 0 ? hits[index] : undefined;
  useEffect(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!open || !hit || !needle) { highlights?.delete('thread-search'); highlights?.delete('thread-search-active'); return; }
    const messageId = hit.id.slice(0, hit.id.lastIndexOf(':'));
    const element = document.querySelector(`[data-message-id="${CSS.escape(messageId)}"]`);
    element?.scrollIntoView({block: 'center', behavior: 'smooth'});
    if (!highlights || !HighlightCtor) return;
    const all = [...document.querySelectorAll('.thread [data-message-id]')].flatMap(node => rangesIn(node, needle));
    // The rendered text can differ from the raw one, so the active mark is the n-th rendered match in that message.
    const nth = hits.slice(0, index).filter(other => other.id.startsWith(`${messageId}:`)).length;
    const current = element ? rangesIn(element, needle)[nth] : undefined;
    highlights.set('thread-search', new HighlightCtor(...all));
    if (current) highlights.set('thread-search-active', new HighlightCtor(current)); else highlights.delete('thread-search-active');
  }, [open, hit, hits, index, query]);
  useEffect(() => () => { highlights?.delete('thread-search'); highlights?.delete('thread-search-active'); }, []);

  const close = () => { setOpen(false); setQuery(''); };
  return <div ref={root} className="thread-search" hidden={!open}>
    {open && <ConversationSearch query={query} hits={hits} activeIndex={index} inputRef={input}
      onQueryChange={setQuery} onStep={delta => setActive(value => value + delta)} onSelect={setActive} onClose={close}/>}
  </div>;
}
