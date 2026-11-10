import React,{useEffect,useState} from 'react';
import {invoke} from '../../client';
import type {JsonValue,Provider} from '../../../../../packages/client/src/types';
import {Plus, Search, Trash2} from 'lucide-react';
import {Button} from '../../components/ui/button';
import {Input} from '../../components/ui/input';
import {Textarea} from '../../components/ui/textarea';
import {Select,SelectContent,SelectItem,SelectTrigger,SelectValue} from '../../components/ui/select';
type Row=Record<string,JsonValue>;
type Props={sessionId:string;userScope:boolean;busy:boolean;call:(method:string,payload:Row)=>Promise<JsonValue>;refresh:()=>Promise<void>;report:(error:string)=>void};
export function CurationForms({sessionId,userScope,busy,call,refresh,report}:Props){
 const [providers,setProviders]=useState<Provider[]>([]),[executor,setExecutor]=useState(''),[review1,setReview1]=useState(''),[review2,setReview2]=useState(''),[trigger,setTrigger]=useState(''),[triggerExpected,setTriggerExpected]=useState(''),[holdouts,setHoldouts]=useState([{input:'',expected:''}]),[budget,setBudget]=useState(6);
 const [skill,setSkill]=useState(''),[base,setBase]=useState(''),[candidate,setCandidate]=useState(''),[diff,setDiff]=useState(''),[rationale,setRationale]=useState(''),[applicability,setApplicability]=useState(''),[negative,setNegative]=useState(''),[query,setQuery]=useState(''),[sources,setSources]=useState<Row[]>([]),[source,setSource]=useState('');
 useEffect(()=>{let active=true;invoke('model_providers').then(p=>{if(active)setProviders(p);}).catch(e=>report(String(e)));return()=>{active=false;};},[report]);
 const options=providers.flatMap(p=>p.models.map(m=>({value:`api:${p.id}/${m}`,label:`${p.name} / ${m}`})));
 const selector=(label:string,value:string,set:(v:string)=>void)=><label>{label}<Select value={value||undefined} onValueChange={set}><SelectTrigger className="provider-select" aria-label={label}><SelectValue placeholder="Выберите модель"/></SelectTrigger><SelectContent>{options.map(o=><SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>)}</SelectContent></Select></label>;
 const modelsReady=!!executor&&!!review1&&!!review2;
 async function configure(e:React.FormEvent){e.preventDefault();try{await call('configure',{config:{executorModel:executor,reviewers:[review1,review2],trigger:{input:trigger,contains:[triggerExpected],notContains:[]},holdouts:holdouts.map(x=>({input:x.input,contains:[x.expected],notContains:[]})),maxCalls:budget}});report('');}catch(err){report(String(err));}}
 async function find(){try{const result=await invoke('plugins.call',{id:'session-query',method:'search',params:{sessionId,...(userScope?{scope:{kind:'user'}}:{}),payload:{query}}});const r=result as Row;const list=Array.isArray(result)?result:Array.isArray(r?.hits)?r.hits:Array.isArray(r?.items)?r.items:Array.isArray(r?.results)?r.results:[];setSources(list.filter((v):v is Row=>!!v && typeof v==='object' && !Array.isArray(v)));}catch(e){report(String(e));}}
 async function propose(e:React.FormEvent){e.preventDefault();try{const item=sources[Number(source)];const ref=item?.source ?? item?.sourceRef;if(!ref)throw Error('Выберите источник из результатов поиска');const scope:JsonValue=userScope?{kind:'user'}:{kind:'session',sessionId};await call('propose',{proposal:{id:crypto.randomUUID(),scope,actor:'',sources:[ref],skill,base_revision:base,candidate,diff,rationale,applicability,negative_example:negative,validators:[],reviewers:[],state:'draft',evidence:[],published_revision:null}});await refresh();}catch(e){report(String(e));}}
 return <>
 <form className="provider-form" onSubmit={configure}><h2>Проверки до предложения</h2><p className="ds-field-hint">Модели и примеры, на которых предложение проверяется перед публикацией.</p>
  <div className="ds-field-row">{selector('Исполнитель',executor,setExecutor)}{selector('Рецензент 1',review1,setReview1)}{selector('Рецензент 2',review2,setReview2)}</div>
  <h3 className="ds-section-title">Исходный пример</h3>
  <label>Задача<Textarea required value={trigger} onChange={e=>setTrigger(e.target.value)}/></label>
  <label>Ожидаемый текст<Input required value={triggerExpected} onChange={e=>setTriggerExpected(e.target.value)}/></label>
  {holdouts.map((h,i)=><fieldset key={i} className="learning-fieldset"><legend>Контрольный пример {i+1}</legend><label>Задача<Textarea required value={h.input} onChange={e=>setHoldouts(v=>v.map((x,n)=>n===i?{...x,input:e.target.value}:x))}/></label><label>Ожидаемый текст<Input required value={h.expected} onChange={e=>setHoldouts(v=>v.map((x,n)=>n===i?{...x,expected:e.target.value}:x))}/></label>{holdouts.length>1&&<Button type="button" variant="ghost" size="sm" onClick={()=>setHoldouts(v=>v.filter((_,n)=>n!==i))}><Trash2/> Удалить пример</Button>}</fieldset>)}
  <Button type="button" variant="outline" onClick={()=>setHoldouts(v=>[...v,{input:'',expected:''}])}><Plus/> Добавить пример</Button>
  <label>Лимит вызовов<Input type="number" min={2*(holdouts.length+1)+2} max="100" required value={budget} onChange={e=>setBudget(Number(e.target.value))}/><span className="ds-field-hint">Не меньше {2*(holdouts.length+1)+2}: по два вызова на каждый пример и рецензии.</span></label>
  <div className="ds-form-actions"><Button disabled={busy || !sessionId || !modelsReady}>Сохранить проверки</Button></div>
 </form>
 <form className="provider-form" onSubmit={propose}><h2>Предложение изменения навыка</h2>
  <h3 className="ds-section-title">Источник</h3>
  <div className="learning-search"><label>Найти в истории<Input type="search" value={query} onChange={e=>setQuery(e.target.value)}/></label><Button type="button" variant="outline" onClick={()=>void find()} disabled={!query || busy}><Search/> Искать</Button></div>
  <label>Источник<Select value={source||undefined} onValueChange={setSource} disabled={!sources.length}><SelectTrigger className="provider-select" aria-label="Источник"><SelectValue placeholder={sources.length?'Выберите источник':'Сначала найдите источник'}/></SelectTrigger><SelectContent>{sources.map((s,i)=><SelectItem key={i} value={String(i)}>{String(s.text ?? s.snippet ?? `Источник ${i+1}`)}</SelectItem>)}</SelectContent></Select></label>
  <h3 className="ds-section-title">Навык</h3>
  <div className="ds-field-row"><label>Навык<Input required value={skill} onChange={e=>setSkill(e.target.value)}/></label><label>Базовая ревизия<Input required value={base} onChange={e=>setBase(e.target.value)}/></label></div>
  <label>Новое содержимое<Textarea required className="learning-code" value={candidate} onChange={e=>setCandidate(e.target.value)}/></label>
  <label>Описание изменений<Textarea required value={diff} onChange={e=>setDiff(e.target.value)}/></label>
  <label>Обоснование<Textarea required value={rationale} onChange={e=>setRationale(e.target.value)}/></label>
  <div className="ds-field-row"><label>Применимость<Input required value={applicability} onChange={e=>setApplicability(e.target.value)}/></label><label>Отрицательный пример<Input required value={negative} onChange={e=>setNegative(e.target.value)}/></label></div>
  <div className="ds-form-actions"><Button disabled={busy || !sessionId || !source}>Создать предложение</Button></div>
 </form></>;
}
