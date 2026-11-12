export type TextSegment = {kind: 'text'; text: string} | {kind: 'code'; language: string; code: string};
const shells = new Set(['sh', 'bash', 'zsh', 'fish', 'shell', 'console', 'terminal', 'powershell', 'pwsh', 'ps1', 'cmd', 'bat']);
/** Slot key for a fence language: every shell dialect shares `shell`, so one terminal renderer covers them. */
export const codeSlotKey = (language: string) => shells.has(language.toLowerCase()) ? 'shell' : language.toLowerCase() || 'text';
/** Splits fenced ``` blocks out of plain message text; an unclosed fence (still streaming) stays text. */
export function splitCodeBlocks(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  const fence = /```([\w+#.-]*)[^\S\n]*\n([\s\S]*?)\n?```/g;
  let last = 0;
  for (let match = fence.exec(text); match; match = fence.exec(text)) {
    if (match.index > last) segments.push({kind: 'text', text: text.slice(last, match.index)});
    segments.push({kind: 'code', language: match[1], code: match[2]});
    last = match.index + match[0].length;
  }
  if (last < text.length) segments.push({kind: 'text', text: text.slice(last)});
  return segments;
}
