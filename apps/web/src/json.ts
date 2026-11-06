import type {JsonValue} from '../../../packages/client/src/generated.ts';
export const jsonObject=(value:JsonValue):{[key:string]:JsonValue}|undefined=>value!==null&&typeof value==='object'&&!Array.isArray(value)?value:undefined;
/** Match the JSON serialization used on the wire for UI-only extension metadata. */
export const jsonSnapshot=(value:unknown):JsonValue=>JSON.parse(JSON.stringify(value)) as JsonValue;
