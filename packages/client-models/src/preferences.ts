export type PreferenceScope={server:string;principal:string;workspace:string};
export class Preferences {
 private prefix:string;
 private storage:Storage;
 constructor(storage:Storage,scope:PreferenceScope){this.storage=storage;this.prefix=`minerva:${encodeURIComponent(scope.server)}:${encodeURIComponent(scope.principal)}:${encodeURIComponent(scope.workspace)}:`;}
 get<T>(key:string,fallback:T):T{try{return JSON.parse(this.storage.getItem(this.prefix+key)||'null')??fallback;}catch{return fallback;}}
 set(key:string,value:unknown){this.storage.setItem(this.prefix+key,JSON.stringify(value));}
 clear(){for(let i=this.storage.length-1;i>=0;i--){const key=this.storage.key(i);if(key?.startsWith(this.prefix))this.storage.removeItem(key);}}
}
