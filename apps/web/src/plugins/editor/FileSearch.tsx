import React, {useEffect, useState} from 'react';
import {File, Search, X} from 'lucide-react';
import {invoke} from '../../client';
import {Input} from '../../components/ui/input';
import {Button} from '../../components/ui/button';
import {ScrollArea} from '../../components/ui/scroll-area';
import {open, openAt} from './store';

type Result = {files: string[]; matches: {path: string; line: number; column: number; text: string}[]; truncated: boolean};

function Highlight({text, query}: {text: string; query: string}) {
  const at = text.toLocaleLowerCase().indexOf(query.toLocaleLowerCase());
  if (at < 0 || !query) return <>{text}</>;
  return <>{text.slice(0, at)}<mark>{text.slice(at, at + query.length)}</mark>{text.slice(at + query.length)}</>;
}
const split = (path: string) => { const at = path.lastIndexOf('/'); return {name: path.slice(at + 1), dir: at > 0 ? path.slice(0, at) : ''}; };

/** File-name and content search for one workspace; results open files in the editor at the matching line. */
export function FileSearch({view, owner, onClose}: {view: string; owner: string; onClose: () => void}) {
  const [query, setQuery] = useState('');
  const [result, setResult] = useState<Result | null>(null);
  const [state, setState] = useState<'idle' | 'loading' | 'error'>('idle');
  const [error, setError] = useState('');
  useEffect(() => {
    const needle = query.trim();
    if (!needle) { setResult(null); setState('idle'); return; }
    let alive = true;
    setState('loading');
    // Debounced so typing does not walk the workspace on every key.
    const timer = setTimeout(() => {
      void invoke('mh-editor.search', {sessionId: owner, query: needle})
        .then(value => { if (alive) { setResult(value); setState('idle'); } })
        .catch(cause => { if (alive) { setError(String(cause).replace(/^MinervaError: /, '')); setState('error'); } });
    }, 250);
    return () => { alive = false; clearTimeout(timer); };
  }, [query, owner]);
  const groups = new Map<string, Result['matches']>();
  for (const match of result?.matches ?? []) groups.set(match.path, [...(groups.get(match.path) ?? []), match]);
  const needle = query.trim();
  return <div className="file-search">
    <div className="file-search-field">
      <Search size={14} aria-hidden="true"/>
      <Input autoFocus type="search" placeholder="Поиск по файлам и содержимому" aria-label="Поиск по файлам" value={query}
        onChange={event => setQuery(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); onClose(); } }}/>
      <Button variant="ghost" size="icon-sm" aria-label="Закрыть поиск" onClick={onClose}><X/></Button>
    </div>
    <ScrollArea className="file-search-results" aria-live="polite">
      {state === 'loading' && <p className="editor-hint" role="status">Ищем…</p>}
      {state === 'error' && <p className="editor-error" role="alert">{error}</p>}
      {state === 'idle' && result && !result.files.length && !result.matches.length && <p className="editor-hint">Ничего не найдено</p>}
      {state === 'idle' && result && result.files.length > 0 && <section aria-label="Файлы">
        <h3 className="file-search-heading">Файлы · {result.files.length}</h3>
        {result.files.map(path => { const {name, dir} = split(path); return <button key={path} type="button" className="file-search-item" title={path}
          onClick={() => void open(view, path, false, owner)} onDoubleClick={() => void open(view, path, true, owner)}>
          <File size={14} aria-hidden="true"/><span className="file-search-name"><Highlight text={name} query={needle}/></span>{dir && <span className="file-search-dir">{dir}</span>}
        </button>; })}
      </section>}
      {state === 'idle' && groups.size > 0 && <section aria-label="Совпадения в содержимом">
        <h3 className="file-search-heading">Совпадения · {result!.matches.length}</h3>
        {[...groups].map(([path, lines]) => <div key={path} className="file-search-group">
          <div className="file-search-file" title={path}><File size={14} aria-hidden="true"/><span className="file-search-name">{split(path).name}</span><span className="file-search-dir">{split(path).dir}</span><span className="file-search-count">{lines.length}</span></div>
          {lines.map(match => <button key={`${match.line}:${match.column}`} type="button" className="file-search-line" onClick={() => void openAt(view, path, owner, match.line, match.column)}>
            <span className="file-search-lineno">{match.line}</span><span className="file-search-text"><Highlight text={match.text.trim()} query={needle}/></span>
          </button>)}
        </div>)}
      </section>}
      {state === 'idle' && result?.truncated && <p className="editor-hint">Показаны не все совпадения — уточните запрос.</p>}
    </ScrollArea>
  </div>;
}
