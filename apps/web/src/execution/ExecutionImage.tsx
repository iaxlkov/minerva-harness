import type {ImageMessagePartProps} from '@assistant-ui/react';
import {object} from './toolModel';
import {Image} from '../components/assistant-ui/elements/image';
import './execution.css';

export function ExecutionImage(props:ImageMessagePartProps) {
  const attachment=object(object(props.providerMetadata).minerva).attachment;
  const name=object(attachment).name;
  return <Image {...props} filename={typeof name==='string'?name:props.filename}/>;
}
