export const isMarkdown = (path: string) => /\.(md|markdown|mdx)$/i.test(path);
/** Workspace path for a relative link in `from`; null for external, anchor-only or escaping links. */
export function resolveLink(from: string, href: string): string | null {
  const target = decodeURIComponent(href.split(/[?#]/)[0] ?? '');
  if (!target || /^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith('//')) return null;
  const parts = target.startsWith('/') ? [] : from.split('/').slice(0, -1);
  for (const part of target.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') { if (!parts.length) return null; parts.pop(); } else parts.push(part);
  }
  return parts.join('/') || null;
}
