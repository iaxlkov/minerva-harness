import type {SearchHit} from '../../components/assistant-ui/elements/conversation-search';

export type SearchableMessage = {id: string; text: string};
/** Case-insensitive matches across messages, in reading order, with context for the hit list. */
export function findHits(messages: readonly SearchableMessage[], query: string, limit = 200, context = 40): SearchHit[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [];
  const hits: SearchHit[] = [];
  messages.forEach((message, index) => {
    const haystack = message.text.toLocaleLowerCase();
    // Lower-casing can change length for a few scripts; fall back to the original text then.
    const source = haystack.length === message.text.length ? message.text : haystack;
    for (let at = haystack.indexOf(needle); at !== -1 && hits.length < limit; at = haystack.indexOf(needle, at + needle.length)) {
      const start = Math.max(0, at - context), end = Math.min(source.length, at + needle.length + context);
      hits.push({
        id: `${message.id}:${at}`,
        before: (start > 0 ? '…' : '') + source.slice(start, at).replace(/\s+/g, ' '),
        match: source.slice(at, at + needle.length),
        after: source.slice(at + needle.length, end).replace(/\s+/g, ' ') + (end < source.length ? '…' : ''),
        position: messages.length > 1 ? (index / (messages.length - 1)) * 96 : 0,
      });
    }
  });
  return hits;
}
