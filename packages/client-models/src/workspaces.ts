import {Store} from './store.ts';
import type {Invoke,WorkspaceInfo} from '../../client/src/types.ts';

export class WorkspacesModel extends Store<WorkspaceInfo[]> {
 private invoke:Invoke;
 constructor(invoke:Invoke){super([]);this.invoke=invoke;}
 refresh=()=>this.load(()=>this.invoke('workspaces.list'));
 async rename(id:string,title:string){
  const generation=this.generation;
  await this.invoke('workspaces.rename',{id,title});
  if(generation!==this.generation)throw Error('Client scope changed');
  await this.refresh();
 }
 async remove(id:string){
  const generation=this.generation;
  await this.invoke('workspaces.remove',{id});
  if(generation!==this.generation)throw Error('Client scope changed');
  await this.refresh();
 }
 async create(path:string){
  const generation=this.generation;
  const workspace=await this.invoke('workspaces.create',{path});
  if(generation!==this.generation)throw Error('Client scope changed');
  this.set([workspace,...this.state.filter(item=>item.id!==workspace.id)]);
  return workspace;
 }
 dispose=()=>this.clear([]);
}
