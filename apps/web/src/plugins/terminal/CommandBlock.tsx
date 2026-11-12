import React, {useState} from 'react';
import {Check, Copy, Play, TerminalSquare} from 'lucide-react';
import type {AppSlotProps} from '../../app/plugins';
import {canvas} from '../../app/canvas';
import {Button} from '../../components/ui/button';
import {sendToTerminal} from './store';

/** A shell snippet from the chat with explicit Run: nothing executes until the user clicks. */
export function CommandBlock({code, language, sessionId}: AppSlotProps['message.code']) {
  const [copied, setCopied] = useState(false);
  const [ran, setRan] = useState(false);
  const run = () => {
    // Each line is typed into the terminal as if entered by hand.
    sendToTerminal(sessionId, code.split('\n').filter(line => line.trim()).map(line => `${line}\r`).join(''));
    canvas.open('mh-terminal');
    setRan(true);
  };
  const copy = () => void navigator.clipboard?.writeText(code).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500); });
  return <figure className="command-block" data-language={language || undefined}>
    <figcaption className="command-block-head">
      <TerminalSquare aria-hidden="true"/><span>{language || 'shell'}</span>
      <Button variant="ghost" size="icon-sm" aria-label="Копировать команду" title="Копировать" onClick={copy}>{copied ? <Check/> : <Copy/>}</Button>
      <Button variant={ran ? 'outline' : 'default'} size="sm" onClick={run}><Play/>{ran ? 'Выполнить снова' : 'Выполнить'}</Button>
    </figcaption>
    <pre className="command-block-code">{code}</pre>
  </figure>;
}
