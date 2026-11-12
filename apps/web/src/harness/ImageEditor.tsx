import React, {useCallback, useEffect, useRef, useState} from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import {Eraser, MessageSquarePlus, PenLine, Scaling, Undo2, X} from 'lucide-react';
import {Button} from '../components/ui/button';
import {DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger} from '../components/ui/dropdown-menu';
import {UiSlot, workspaceScope} from '../app/plugins';
import {commentsText, markAt, scaledSize, type Mark, type Point} from './imageEdit';
import './image-editor.css';

type Tool = 'markup' | 'comment' | 'erase';
const colors = ['#ef4444', '#f59e0b', '#22c55e', '#3b82f6', '#ffffff', '#111111'];
const widths = [{id: 's', label: 'Тонкая', k: 0.003}, {id: 'm', label: 'Средняя', k: 0.006}, {id: 'l', label: 'Толстая', k: 0.012}];
const scales = [1, 0.75, 0.5, 0.25];

function loadImage(blob: Blob) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const image = new Image();
    image.onload = () => { URL.revokeObjectURL(url); resolve(image); };
    image.onerror = () => { URL.revokeObjectURL(url); reject(new Error('Не удалось открыть изображение')); };
    image.src = url;
  });
}
function draw(ctx: CanvasRenderingContext2D, image: HTMLImageElement, marks: readonly Mark[], draft: Mark | null) {
  const {naturalWidth: w, naturalHeight: h} = image;
  ctx.clearRect(0, 0, w, h);
  ctx.drawImage(image, 0, 0);
  ctx.lineCap = 'round'; ctx.lineJoin = 'round';
  const radius = Math.max(12, Math.min(w, h) * 0.022);
  let pin = 0;
  for (const mark of draft ? [...marks, draft] : marks) {
    if (mark.kind === 'stroke') {
      ctx.strokeStyle = mark.color; ctx.lineWidth = mark.width; ctx.beginPath();
      mark.points.forEach((p, i) => i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y));
      if (mark.points.length === 1) ctx.lineTo(mark.points[0].x + 0.1, mark.points[0].y);
      ctx.stroke();
      continue;
    }
    pin++;
    ctx.fillStyle = '#f59e0b'; ctx.strokeStyle = '#1c1b1a'; ctx.lineWidth = radius * 0.18;
    ctx.beginPath(); ctx.arc(mark.at.x, mark.at.y, radius, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
    ctx.fillStyle = '#1c1b1a'; ctx.font = `600 ${radius * 1.1}px Inter, sans-serif`; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.fillText(String(pin), mark.at.x, mark.at.y + radius * 0.05);
  }
}

/** Composer image editor: markup, numbered comments, eraser for marks and downscaling before sending. */
export function ImageEditor({file, name, open, onOpenChange, onApply}: {file: File; name: string; open: boolean; onOpenChange: (open: boolean) => void; onApply: (file: File, comments: string) => void}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [source, setSource] = useState<Blob>(file);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [draft, setDraft] = useState<Mark | null>(null);
  const [tool, setTool] = useState<Tool>('markup');
  const [color, setColor] = useState(colors[0]);
  const [width, setWidth] = useState('m');
  const [scale, setScale] = useState(1);
  const [editing, setEditing] = useState<{index: number; left: number; top: number} | null>(null);
  const [error, setError] = useState('');

  useEffect(() => { if (open) { setSource(file); setMarks([]); setScale(1); setEditing(null); } }, [open, file]);
  useEffect(() => {
    if (!open) return;
    let alive = true;
    void loadImage(source).then(value => { if (alive) setImage(value); }, cause => { if (alive) setError(String(cause)); });
    return () => { alive = false; };
  }, [open, source]);
  useEffect(() => {
    const ctx = canvas.current?.getContext('2d');
    if (ctx && image) draw(ctx, image, marks, draft);
  }, [image, marks, draft]);

  const point = (event: React.PointerEvent): Point => {
    const rect = canvas.current!.getBoundingClientRect();
    return {x: (event.clientX - rect.left) * (image!.naturalWidth / rect.width), y: (event.clientY - rect.top) * (image!.naturalHeight / rect.height)};
  };
  const eraseRadius = () => Math.max(8, Math.min(image!.naturalWidth, image!.naturalHeight) * 0.02);
  const erase = (p: Point) => setMarks(current => { const i = markAt(current, p, eraseRadius()); return i < 0 ? current : current.filter((_, n) => n !== i); });
  const down = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (!image) return;
    const p = point(event);
    if (tool === 'comment') {
      const stage = canvas.current!.parentElement!.getBoundingClientRect();
      setEditing({index: marks.length, left: event.clientX - stage.left, top: event.clientY - stage.top});
      setMarks(current => [...current, {kind: 'pin', at: p, text: ''}]);
      return;
    }
    event.currentTarget.setPointerCapture(event.pointerId);
    if (tool === 'erase') { erase(p); return; }
    const k = widths.find(item => item.id === width)!.k;
    setDraft({kind: 'stroke', color, width: Math.max(2, Math.min(image.naturalWidth, image.naturalHeight) * k), points: [p]});
  };
  const move = (event: React.PointerEvent<HTMLCanvasElement>) => {
    if (!image || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
    const p = point(event);
    if (tool === 'erase') erase(p);
    else setDraft(current => current?.kind === 'stroke' ? {...current, points: [...current.points, p]} : current);
  };
  const up = () => { if (draft) { setMarks(current => [...current, draft]); setDraft(null); } };
  const undo = useCallback(() => { setEditing(null); setMarks(current => current.slice(0, -1)); }, []);
  useEffect(() => {
    if (!open) return;
    const key = (event: KeyboardEvent) => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z' && !(event.target instanceof HTMLInputElement)) { event.preventDefault(); undo(); } };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [open, undo]);

  /** The current picture with marks burnt in, at the chosen output size. */
  const render = (outputScale = scale) => new Promise<Blob>((resolve, reject) => {
    if (!image) return reject(new Error('Изображение ещё загружается'));
    const full = document.createElement('canvas');
    full.width = image.naturalWidth; full.height = image.naturalHeight;
    draw(full.getContext('2d')!, image, marks, null);
    const size = scaledSize(full.width, full.height, outputScale);
    const out = document.createElement('canvas');
    out.width = size.width; out.height = size.height;
    const ctx = out.getContext('2d')!;
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(full, 0, 0, size.width, size.height);
    out.toBlob(blob => blob ? resolve(blob) : reject(new Error('Не удалось сохранить изображение')), 'image/png');
  });
  const apply = async () => {
    try {
      const blob = await render();
      const base = name.replace(/\.[^.]+$/, '') || 'image';
      onApply(new File([blob], `${base}.png`, {type: 'image/png'}), commentsText(name, marks));
      onOpenChange(false);
    } catch (cause) { setError(String(cause)); }
  };
  const output = image ? scaledSize(image.naturalWidth, image.naturalHeight, scale) : null;
  const toolButton = (id: Tool, Icon: typeof PenLine, label: string) =>
    <Button variant="ghost" size="sm" aria-pressed={tool === id} onClick={() => { setTool(id); setEditing(null); }}><Icon/>{label}</Button>;

  return <Dialog.Root open={open} onOpenChange={onOpenChange}>
    <Dialog.Portal>
      <Dialog.Overlay className="image-editor-overlay"/>
      <Dialog.Content className="oe-studio image-editor" aria-describedby={undefined}>
        <Dialog.Title className="oe-sr-only">Редактирование {name}</Dialog.Title>
        <div className="image-editor-stage">
          {image ? <canvas ref={canvas} width={image.naturalWidth} height={image.naturalHeight} className={`image-editor-canvas tool-${tool}`}
            onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}/> : <p role="status">{error || 'Загрузка…'}</p>}
          {editing && marks[editing.index]?.kind === 'pin' && <input autoFocus className="image-editor-comment" style={{left: editing.left, top: editing.top}} placeholder={`Комментарий ${marks.slice(0, editing.index + 1).filter(m => m.kind === 'pin').length}`} aria-label="Текст комментария"
            value={(marks[editing.index] as Extract<Mark, {kind: 'pin'}>).text}
            onChange={event => { const text = event.target.value; setMarks(current => current.map((m, i) => i === editing.index && m.kind === 'pin' ? {...m, text} : m)); }}
            onKeyDown={event => { if (event.key === 'Enter' || event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setEditing(null); } }}/>}
          <Dialog.Close asChild><Button variant="ghost" size="icon-lg" className="image-editor-close" aria-label="Закрыть без изменений"><X/></Button></Dialog.Close>
        </div>
        {error && image && <p role="alert" className="image-editor-error">{error}</p>}
        <div className="image-editor-toolbar" role="toolbar" aria-label="Инструменты изображения">
          {toolButton('markup', PenLine, 'Разметка')}
          {toolButton('comment', MessageSquarePlus, 'Комментировать')}
          {toolButton('erase', Eraser, 'Стереть')}
          <DropdownMenu>
            <DropdownMenuTrigger asChild><Button variant="ghost" size="sm"><Scaling/>Размер{scale < 1 ? ` ${scale * 100}%` : ''}</Button></DropdownMenuTrigger>
            <DropdownMenuContent side="top" align="center">
              <DropdownMenuLabel>{output ? `Итог: ${output.width}×${output.height}` : 'Размер'}</DropdownMenuLabel>
              <DropdownMenuRadioGroup value={String(scale)} onValueChange={value => setScale(Number(value))}>
                {scales.map(value => <DropdownMenuRadioItem key={value} value={String(value)}>{value * 100}%</DropdownMenuRadioItem>)}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
          <UiSlot slot="composer.image.tools" scope={workspaceScope} componentProps={{name, render: () => render(1), replace: (blob: Blob) => { setMarks([]); setSource(blob); }}}/>
          {tool === 'markup' && <span className="image-editor-options">
            {colors.map(value => <button key={value} type="button" className="image-editor-swatch" style={{background: value}} aria-label={`Цвет ${value}`} aria-pressed={color === value} onClick={() => setColor(value)}/>)}
            {widths.map(item => <button key={item.id} type="button" className="image-editor-width" aria-label={item.label} title={item.label} aria-pressed={width === item.id} onClick={() => setWidth(item.id)}><span style={{height: item.id === 's' ? 2 : item.id === 'm' ? 4 : 7}}/></button>)}
          </span>}
          <span className="image-editor-separator" aria-hidden="true"/>
          <Button variant="ghost" size="icon-sm" aria-label="Отменить последнее действие" title="Отменить (Ctrl+Z)" disabled={!marks.length} onClick={undo}><Undo2/></Button>
          <Button size="sm" disabled={!image} onClick={() => void apply()}>Готово</Button>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
