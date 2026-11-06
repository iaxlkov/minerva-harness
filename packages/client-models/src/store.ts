/** React-free observable state. Epochs fence async responses across updates and disposal. */
export class Store<T> {
 private listeners = new Set<()=>void>();
 protected revision=0;
 protected generation=0;
 protected state:T;
 constructor(state:T) {this.state=state;}
 getSnapshot=()=>this.state;
 subscribe=(listener:()=>void)=>{this.listeners.add(listener);return ()=>{this.listeners.delete(listener);};};
 set=(value:T)=>{this.revision++;this.state=value;for(const listener of this.listeners)listener();};
 clear=(value:T)=>{this.generation++;this.set(value);this.listeners.clear();};
 async load(fetch:()=>Promise<T>){const revision=++this.revision;const value=await fetch();if(revision===this.revision)this.set(value);}
}
