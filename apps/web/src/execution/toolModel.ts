type RecordValue = Record<string, unknown>;
export const object = (value:unknown):RecordValue => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : {};
export const display = (value:unknown):string => typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? '';
const states = {pending:'Ожидание', running:'Выполняется', 'requires-approval':'Требуется разрешение', completed:'Завершено', failed:'Ошибка', cancelled:'Отменено', interrupted:'Прервано'};
export function toolState(part:{result?:unknown;isError?:boolean;providerMetadata?:unknown;status?:{type:string;reason?:string}}) {
  const recorded=object(object(part.providerMetadata).minerva).status;
  const interrupted=part.status?.type==='incomplete' && ['pending','running','requires-approval'].includes(String(recorded));
  const status = interrupted && part.result===undefined ? part.status?.reason==='cancelled'?'cancelled':'interrupted'
    : typeof recorded === 'string' && Object.hasOwn(states, recorded) ? recorded as keyof typeof states
    : part.result !== undefined ? part.isError?'failed':'completed'
    : part.status?.type==='incomplete' ? part.status.reason==='cancelled'?'cancelled':'interrupted'
    : part.status?.type==='running'?'running':'pending';
  return {status,label:states[status]};
}
export function toolCard(name:string,args:unknown,output:unknown) {
  const input=object(args);
  const item=Array.isArray(output)&&output.length===1?object(output[0]):null;
  const result=object(item?.type==='json'?item.value:output);
  const tool=name.split('.').at(-1) ?? name;
  const kind = ['shell','bash','pwsh','execute','exec'].includes(tool)?'shell'
    : ['read','read_file'].includes(tool)?'read'
    : ['search','grep','glob','search_files'].includes(tool)?'search'
    : ['patch','patch_file','apply_patch','write','write_file','edit'].includes(tool)?'diff':'generic';
  return {kind,input,result,output};
}
