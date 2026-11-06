import {Store} from './store.ts';
import type {ContentPart} from '../../client/src/types.ts';
export type RunSnapshot={runId:string|null;parts:ContentPart[];running:boolean};
export class RunsModel extends Store<RunSnapshot> {
 constructor(){super({runId:null,parts:[],running:false});}
 baseline=(value:RunSnapshot)=>this.set(value);
 append=(part:ContentPart)=>this.set({...this.state,parts:[...this.state.parts,part]});
 dispose=()=>this.clear({runId:null,parts:[],running:false});
}
