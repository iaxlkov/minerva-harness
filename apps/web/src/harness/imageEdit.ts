export type Point = {x: number; y: number};
export type Stroke = {kind: 'stroke'; color: string; width: number; points: Point[]};
export type Pin = {kind: 'pin'; at: Point; text: string};
export type Mark = Stroke | Pin;

function segmentDistance(p: Point, a: Point, b: Point) {
  const dx = b.x - a.x, dy = b.y - a.y, length = dx * dx + dy * dy;
  const t = length ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / length)) : 0;
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
}
/** Index of the topmost mark under `p` (strokes by their line, pins by their badge), or -1. */
export function markAt(marks: readonly Mark[], p: Point, radius: number): number {
  for (let i = marks.length - 1; i >= 0; i--) {
    const mark = marks[i];
    if (mark.kind === 'pin') { if (Math.hypot(p.x - mark.at.x, p.y - mark.at.y) <= radius * 1.5) return i; continue; }
    const points = mark.points, reach = radius + mark.width / 2;
    if (points.length === 1 && Math.hypot(p.x - points[0].x, p.y - points[0].y) <= reach) return i;
    for (let j = 1; j < points.length; j++) if (segmentDistance(p, points[j - 1], points[j]) <= reach) return i;
  }
  return -1;
}
/** Output size for a scale, never below 1px and never above the source. */
export function scaledSize(width: number, height: number, scale: number) {
  const factor = Math.max(0.01, Math.min(1, scale));
  return {width: Math.max(1, Math.round(width * factor)), height: Math.max(1, Math.round(height * factor))};
}
/** Message text that carries pin comments, numbered like the badges burnt into the image. */
export function commentsText(name: string, marks: readonly Mark[]) {
  // Numbers follow every pin, as drawn on the image, even when a pin has no text.
  const lines = marks.filter((mark): mark is Pin => mark.kind === 'pin').map((pin, i) => pin.text.trim() && `${i + 1}. ${pin.text.trim()}`).filter(Boolean);
  return lines.length ? `Комментарии к ${name}:\n${lines.join('\n')}` : '';
}
