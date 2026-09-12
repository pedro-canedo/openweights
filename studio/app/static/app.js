'use strict';
const $ = s => document.querySelector(s);
const state = { datasets: [], models: [], jobs: [], system: null, page: '', selected: null, generation: null, runsPage: 1, chat: [], pg: { target:'', system:'Responda em português de forma clara.', max_tokens:200, temperature:0 } };
const titles = {overview:'Visão geral', datasets:'Datasets',models:'Modelos',train:'Novo treinamento',runs:'Experimentos',playground:'Playground',settings:'Configurações'};
const statusText = {registered:'Cadastrado',ready:'Disponível',queued:'Na fila',running:'Em execução',completed:'Concluído',failed:'Falhou',cancelled:'Cancelado',cancelling:'Cancelando',interrupted:'Interrompido',preparing:'Preparando'};
const modes = {qlora:'QLoRA · instruções',continued:'QLoRA · texto livre',scratch:'Do zero · LLM',scratch_moe:'Do zero · MoE'};
const esc = s => String(s ?? '').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const num = n => Number(n || 0).toLocaleString('pt-BR');
const fixed = (n,d=2) => n==null?'—':Number(n).toLocaleString('pt-BR',{maximumFractionDigits:d});
const ICONS={
 grid:'<rect x="3.5" y="3.5" width="7" height="7" rx="1.8"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.8"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.8"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.8"/>',
 database:'<ellipse cx="12" cy="5.6" rx="7.4" ry="2.9"/><path d="M4.6 5.6v12.8c0 1.6 3.3 2.9 7.4 2.9s7.4-1.3 7.4-2.9V5.6"/><path d="M4.6 12c0 1.6 3.3 2.9 7.4 2.9s7.4-1.3 7.4-2.9"/>',
 layers:'<path d="M12 3.2l8.8 4.9-8.8 4.9-8.8-4.9L12 3.2z"/><path d="M3.2 13.4l8.8 4.9 8.8-4.9"/>',
 train:'<path d="M6.5 17.5L17.5 6.5"/><path d="M9 6.5h8.5V15"/>',
 runs:'<path d="M3 12h3.6l2.7 7.5 4-15 2.7 7.5H21"/>',
 terminal:'<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M7.5 9.5l3 2.5-3 2.5"/><path d="M12.5 14.5h4.5"/>',
 plus:'<path d="M12 5.5v13M5.5 12h13"/>',
 arrow:'<path d="M4 12h15.5"/><path d="M13.5 5.5L20 12l-6.5 6.5"/>',
 download:'<path d="M12 4v11.5"/><path d="M6.5 10.5L12 16l5.5-5.5"/><path d="M4.5 20h15"/>',
 upload:'<path d="M12 16V4.5"/><path d="M6.5 9.5L12 4l5.5 5.5"/><path d="M4.5 20h15"/>',
 x:'<path d="M6 6l12 12M18 6L6 18"/>',
 chip:'<rect x="6.5" y="6.5" width="11" height="11" rx="2"/><path d="M9.5 3v3.5M14.5 3v3.5M9.5 17.5V21M14.5 17.5V21M3 9.5h3.5M3 14.5h3.5M17.5 9.5H21M17.5 14.5H21"/>',
 flask:'<path d="M9.8 3h4.4"/><path d="M10.5 3v5L5 17.6A2.3 2.3 0 0 0 7.1 21h9.8a2.3 2.3 0 0 0 2.1-3.4L13.5 8V3"/><path d="M8 14.5h8"/>',
 folder:'<path d="M4 7.2A2.2 2.2 0 0 1 6.2 5h3.3l1.8 2.2H18A2 2 0 0 1 20 9.2V17a2 2 0 0 1-2 2H6.2A2.2 2.2 0 0 1 4 16.8V7.2z"/>',
 zap:'<path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>',
 inbox:'<path d="M3.5 13.5l2.7-8.1A2 2 0 0 1 8.1 4h7.8a2 2 0 0 1 1.9 1.4l2.7 8.1V17a2.5 2.5 0 0 1-2.5 2.5H6A2.5 2.5 0 0 1 3.5 17v-3.5z"/><path d="M3.5 13.5H9a3 3 0 0 0 6 0h5.5"/>',
 play:'<path d="M7.5 5.2l11 6.8-11 6.8V5.2z"/>',
 gauge:'<path d="M4.5 19a9 9 0 1 1 15 0"/><path d="M12 15l4-5.5"/><circle cx="12" cy="15" r="1.6"/>',
 chevL:'<path d="M15 5.5L8.5 12 15 18.5"/>',
 send:'<path d="M3.6 11.2L20.4 4 13.2 20.6l-1.9-7.3L3.6 11.2z"/><path d="M11.3 13.3L20.4 4"/>',
 chat:'<path d="M5 6.2h14a2 2 0 0 1 2 2V15a2 2 0 0 1-2 2h-6.2L8 20.4V17H5a2 2 0 0 1-2-2V8.2a2 2 0 0 1 2-2z"/>',
};
const ic=(n,s=16)=>`<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[n]||''}</svg>`;
const badge = status => `<span class="tag ${esc(status)}">${esc(statusText[status]||status)}</span>`;
const date = d => d?new Date(d).toLocaleString('pt-BR',{day:'2-digit',month:'short',hour:'2-digit',minute:'2-digit'}):'—';
async function api(url, options={}) {
 await window.studioSessionReady;
 const response = await fetch('/api'+url,options);
 if (!response.ok) { let b;try{b=await response.json()}catch{b={detail:response.statusText}};throw new Error(Array.isArray(b.detail)?b.detail.map(e=>`${e.loc?.slice(1).join('.')}: ${e.msg}`).join('\n'):b.detail||'Falha na solicitação.'); }
 return response.json();
}
const post=(url,body)=>api(url,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body||{})});
let toastTimer;
function toast(text,error=false){const el=$('#toast');el.textContent=text;el.className=error?'error on':'on';clearTimeout(toastTimer);toastTimer=setTimeout(()=>el.classList.remove('on'),error?11000:4500)}
function heading(title,description,action=''){return `<div class="page-title"><div><h1>${title}</h1><p>${description}</p></div>${action}</div>`}
function empty(title,description,button='',icon='inbox'){return `<div class="empty"><div class="empty-icon">${ic(icon,26)}</div><h3>${title}</h3><p>${description}</p>${button}</div>`}
function button(text,action,id='',kind=''){return `<button class="btn ${kind}" data-action="${action}" data-id="${esc(id)}">${text}</button>`}
function runsPageSize(){try{const n=Number(localStorage.getItem('ows.pageSize'));if([5,10,20].includes(n))return n}catch(e){}return 10}
function jobKindLabel(j){
 if(j.kind==='download')return 'Download';
 if(j.kind==='generate')return 'Geração';
 if(j.kind==='gguf')return 'GGUF · '+(j.config?.quantization||'');
 if(j.kind==='train'){
  const m=j.config?.mode;
  if(m==='qlora'||m==='continued')return 'QLoRA';
  if(m==='scratch'||m==='scratch_moe')return 'Do zero';
  return modes[m]||'Treino';
 }
 return j.kind||'';
}
function jobHint(j){
 if(j.status==='running')return 'Em execução na GPU';
 if(j.status==='queued')return 'Aguardando na fila';
 if(j.status==='cancelling')return 'Encerrando o processo';
 if(j.status==='preparing')return 'Preparando dados';
 return '';
}
function pageNums(page,pages){
 if(pages<=7)return Array.from({length:pages},(_,i)=>i+1);
 const set=new Set([1,pages,page,page-1,page+1]);
 if(page<=3){set.add(2);set.add(3);set.add(4)}
 if(page>=pages-2){set.add(pages-3);set.add(pages-2);set.add(pages-1)}
 const arr=[...set].filter(n=>n>=1&&n<=pages).sort((a,b)=>a-b);
 const out=[];
 for(let i=0;i<arr.length;i++){if(i&&arr[i]-arr[i-1]>1)out.push('…');out.push(arr[i])}
 return out;
}
function pagedJobs(jobs){
 const size=runsPageSize();
 const total=jobs.length;
 const pages=Math.max(1,Math.ceil(total/size)||1);
 if(!state.runsPage||state.runsPage<1)state.runsPage=1;
 if(state.runsPage>pages)state.runsPage=pages;
 const start=(state.runsPage-1)*size;
 return {slice:jobs.slice(start,start+size),start,size,total,pages,page:state.runsPage};
}
function runRow(j){
 const selected=j.id===state.selected?' is-selected':'';
 const hint=jobHint(j);
 const live=['running','queued','cancelling','preparing'].includes(j.status);
 return `<article class="run-row${selected}" data-action="job" data-id="${esc(j.id)}" tabindex="0" role="button" aria-pressed="${j.id===state.selected?'true':'false'}">
  <div class="run-row-body">
   <div class="run-kind">${esc(jobKindLabel(j))}</div>
   <span class="run-row-name">${esc(j.name)}</span>
   <div class="run-row-meta"><span>${date(j.created_at)}</span>${hint?`<span class="run-hint">${esc(hint)}</span>`:''}</div>
   ${live?`<div class="run-track" aria-hidden="true"><span></span></div>`:''}
  </div>
  ${badge(j.status)}
  <span class="btn small run-open">Abrir ${ic('arrow',13)}</span>
 </article>`;
}
function pagerHtml(info){
 const {page,pages,start,size,total}=info;
 const from=total?start+1:0;
 const to=Math.min(start+size,total);
 const nums=pageNums(page,pages).map(n=>n==='…'?`<span class="pager-gap" aria-hidden="true">…</span>`:`<button type="button" class="pager-num${n===page?' is-current':''}" data-action="runs-page" data-id="${n}" ${n===page?'aria-current="page" disabled':''}>${n}</button>`).join('');
 const sizes=[5,10,20].map(n=>`<button type="button" class="pager-num${n===size?' is-current':''}" data-action="runs-size" data-id="${n}" aria-label="${n} por página">${n}</button>`).join('');
 return `<div class="pager" role="navigation" aria-label="Paginação dos experimentos">
  <p class="pager-status">Mostrando ${from}–${to} de ${total}</p>
  <div class="pager-pages">
   <button type="button" class="btn small" data-action="runs-page" data-id="${page-1}" ${page<=1?'disabled':''} aria-label="Página anterior">${ic('chevL',14)} Anterior</button>
   ${nums}
   <button type="button" class="btn small" data-action="runs-page" data-id="${page+1}" ${page>=pages?'disabled':''} aria-label="Próxima página">Próxima ${ic('arrow',14)}</button>
  </div>
  <div class="pager-size"><span>Por página</span>${sizes}</div>
 </div>`;
}
function inflightChat(){for(let i=state.chat.length-1;i>=0;i--){const m=state.chat[i];if(m.jobId&&['queued','running','cancelling','preparing'].includes(m.status))return m}return null}
function generating(){return !!inflightChat()}
function chatThreadHtml(){
 if(!state.chat.length)return empty('Sua conversa aparece aqui','Escolha um modelo ao lado e escreva no campo abaixo. A resposta entra nesta conversa.','','chat');
 return state.chat.map(m=>{
  const user=m.role==='user';
  const pending=['queued','running','cancelling','preparing'].includes(m.status);
  const failed=['failed','cancelled'].includes(m.status);
  const body=pending?`<span class="typing" aria-label="O modelo está gerando"><i></i><i></i><i></i></span>`:`<p class="pg-bubble-text">${esc(m.text||'')}</p>${m.meta?`<p class="pg-bubble-meta">${esc(m.meta)}</p>`:''}`;
  return `<div class="pg-msg ${user?'is-user':'is-assistant'}">
   <span class="chat-ava ${user?'user':'model'}" aria-hidden="true">${user?'U':'W'}</span>
   <div class="pg-bubble${failed?' is-error':''}">${body}</div>
  </div>`;
 }).join('');
}
function paintChat(){
 const el=$('#generation-result');
 if(!el)return;
 el.innerHTML=chatThreadHtml();
 el.scrollTop=el.scrollHeight;
}
function syncComposer(){
 const send=$('#pg-send'),cancel=$('#pg-cancel'),ta=$('#pg-prompt'),sel=$('#pg-target');
 if(!send)return;
 const busy=generating();
 const ok=!!(sel&&sel.value&&ta&&ta.value.trim());
 send.disabled=busy||!ok;
 send.hidden=busy;
 if(cancel){
  cancel.hidden=!busy;
  if(busy){
   const current=inflightChat();
   cancel.dataset.id=(current&&current.jobId)||state.generation||'';
  }
 }
}
function rememberPg(){
 const f=$('#generate-form');
 if(!f||!f.elements.target)return;
 state.pg={target:f.elements.target.value,system:f.elements.system.value,max_tokens:f.elements.max_tokens.value,temperature:f.elements.temperature.value};
}
function isMoeJob(j){return j?.training_info?.architecture==='moe'||j?.config?.mode==='scratch_moe'}
function trainVizActive(j){return j&&j.kind==='train'&&['running','queued','cancelling'].includes(j.status)}
function showModal(html){$('#modal-content').innerHTML=html;$('#modal').showModal()}
function modalHeader(title){return `<div class="modal-head"><h2>${title}</h2><button type="button" class="close" data-action="close" aria-label="Fechar">${ic('x',16)}</button></div>`}
async function refresh(){[state.datasets,state.models,state.jobs,state.system]=await Promise.all([api('/datasets'),api('/models'),api('/jobs'),api('/system')]);$('#nav-datasets').textContent=state.datasets.length;$('#nav-models').textContent=state.models.length;const conn=$('#connection');conn.className='connection ok';conn.innerHTML='<i></i> Container conectado';updateMonitor();}
function updateMonitor(){const s=state.system;if(!s)return;const g=s.gpu;const set=(id,pct,text,hot=false)=>{const m=$('#meter-'+id),bar=$('#m-'+id),val=$('#m-'+id+'-v');if(!bar)return;bar.style.width=(pct==null?0:Math.min(100,pct))+'%';if(val)val.textContent=text;if(m)m.classList.toggle('hot',hot);};
 set('gpu',g?.utilization,g?Math.round(g.utilization)+'%':'—',(g?.utilization||0)>92);
 set('vram',g?g.used_mb/g.total_mb*100:null,g?fixed(g.used_mb/1024,1)+'G':'—',g&&g.used_mb/g.total_mb>.92);
 set('cpu',s.cpu_percent,Math.round(s.cpu_percent||0)+'%',(s.cpu_percent||0)>95);
 set('ram',s.ram_total?s.ram_used/s.ram_total*100:null,fixed(s.ram_used/2**30,0)+'G',s.ram_total&&s.ram_used/s.ram_total>.92);
 const mg=$('#meter-gpu');if(mg&&g)mg.title=`${g.name} · ${fixed(g.temperature,0)} °C · ${fixed(g.power_w,0)} W`;}
function route(){const page=location.hash.slice(1)||'overview';state.page=titles[page]?page:'overview';document.querySelectorAll('nav a').forEach(a=>a.classList.toggle('active',a.dataset.page===state.page));$('#page-crumb').textContent=titles[state.page];render()}
function settings(){return heading('Configurações','Preferências e ações avançadas do workspace.')+`<section class="panel danger-zone"><h2>Danger Zone</h2><p>Limpa permanentemente datasets, fontes, modelos cadastrados, preparações, jobs, checkpoints e exports. A aplicação volta ao estado inicial.</p><form id="reset-form"><label class="check"><input type="checkbox" name="ack" required>Entendo que esta ação não pode ser desfeita.</label><label>Digite <code>RESET_WORKSPACE</code> para confirmar<input name="confirmation" autocomplete="off" required pattern="RESET_WORKSPACE"></label><button class="btn danger" type="submit">Limpar todo o workspace</button></form></section>`}
function render(){if(state.page!=='runs')disposeTrainViz();const fn={overview:overview,datasets:datasets,models:models,train:training,runs:runs,playground:playground,settings:settings}[state.page];$('#main').innerHTML=fn();$('#main').classList.toggle('is-playground',state.page==='playground');if(state.page==='runs'&&state.selected)updateDetail();if(state.page==='playground'){syncComposer();const ta=$('#pg-prompt');if(ta){ta.style.height='auto';ta.style.height=Math.min(ta.scrollHeight,200)+'px';ta.focus()}if(state.generation||state.chat.length)updateGeneration()}}
function overview(){const g=state.system?.gpu;const s=state.system;const active=state.jobs.filter(j=>['running','queued','cancelling'].includes(j.status));const completed=state.jobs.filter(j=>j.kind==='train'&&j.status==='completed');const hw=(label,icon,value,pct,detail)=>`<div class="hw"><span class="hw-label">${label} ${ic(icon,13)}</span><b>${value}</b><div class="meter-bar"><span style="width:${pct==null?0:Math.min(100,pct)}%"></span></div><small>${detail}</small></div>`;return heading('Seu próximo modelo começa aqui.','Prepare seus dados, escolha uma base e acompanhe cada etapa.',`<a class="btn primary" href="#train">${ic('plus',15)} Novo treinamento</a>`)+`
 <div class="stats"><div class="stat"><div class="label">MEMÓRIA DA GPU ${ic('chip',14)}</div><strong>${g?fixed(g.used_mb/1024,1):'—'}<span class="unit"> / ${g?fixed(g.total_mb/1024,0):'—'} GB</span></strong><div class="gpu-bar"><span aria-hidden="true" style="width:${g?Math.min(100,g.used_mb/g.total_mb*100):0}%"></span></div><small>${esc(g?.name||'GPU não detectada')}</small></div>
 <div class="stat"><div class="label">DATASETS ${ic('database',14)}</div><strong>${state.datasets.length}</strong><small>${num(state.datasets.reduce((n,d)=>n+d.count,0))} exemplos no workspace</small></div>
 <div class="stat"><div class="label">MODELOS ${ic('layers',14)}</div><strong>${state.models.length}</strong><small>${state.models.filter(m=>m.status==='ready').length} disponíveis localmente</small></div>
 <div class="stat"><div class="label">EXPERIMENTOS ${ic('flask',14)}</div><strong>${completed.length}</strong><small>${active.length} trabalho(s) em execução ou na fila</small></div></div>
 ${state.system?.gpu_error?`<div class="error-inline">${esc(state.system.gpu_error)}</div>`:''}
 <div class="hero"><div><div class="eyebrow">Da ideia ao primeiro checkpoint</div><h2>Treine com seus dados.<br>Construa do seu jeito.</h2><p>Um laboratório para explorar modelos de linguagem, com execução local e configurações pensadas para sua GPU.</p><a href="#datasets" class="btn primary">Adicionar meu primeiro dataset ${ic('arrow',14)}</a></div><div class="pipeline" aria-hidden="true"><div class="node"><b>${ic('database',21)}</b>Dados</div><span>${ic('arrow',14)}</span><div class="node"><b>${ic('layers',21)}</b>Modelo</div><span>${ic('arrow',14)}</span><div class="node"><b>${ic('zap',21)}</b>Seu treino</div></div></div>
 <div class="grid-two"><section class="panel"><div class="panel-head"><div><h2>Atividade recente</h2><p>Acompanhe o que está acontecendo no laboratório.</p></div><a class="btn text" href="#runs">Ver tudo ${ic('arrow',13)}</a></div>${jobTable(state.jobs.slice(0,5))}</section>
 <div><section class="panel"><div class="panel-head"><div><h2>Bancada em tempo real</h2><p>Recursos da fila de treino, downloads e geração.</p></div><span class="tag running">Ao vivo</span></div><div class="hw-grid">${hw('GPU','gauge',g?fixed(g.utilization,0)+'%':'—',g?.utilization,g?esc(g.name):'GPU não detectada')}${hw('VRAM','chip',g?fixed(g.used_mb/1024,1)+' / '+fixed(g.total_mb/1024,0)+' GB':'—',g?g.used_mb/g.total_mb*100:null,g?`${fixed(g.temperature,0)} °C · ${fixed(g.power_w,0)} W`:'—')}${hw('CPU','gauge',s?fixed(s.cpu_percent,0)+'%':'—',s?.cpu_percent,'Processador do host')}${hw('RAM','database',s?fixed(s.ram_used/2**30,0)+' / '+fixed(s.ram_total/2**30,0)+' GB':'—',s?.ram_total?s.ram_used/s.ram_total*100:null,'Memória do container')}</div><div class="system-grid"><span>SSD livre <b>${fixed(s?.disk_free/2**30,0)} GB</b></span><span>Fila <b>${active.length} trabalho(s)</b></span></div></section><section class="panel"><h2>Um caminho simples</h2><div class="steps"><span class="number">01</span><div><h3>Alimente seu dataset</h3><p>Cole textos ou importe arquivos e conversas.</p></div></div><div class="steps"><span class="number">02</span><div><h3>Escolha seu ponto de partida</h3><p>Adapte uma base pronta ou comece do zero.</p></div></div><div class="steps"><span class="number">03</span><div><h3>Treine, observe e experimente</h3><p>Compare métricas e teste o resultado no playground.</p></div></div></section></div></div>`}
function datasets(){return heading('Seus dados, organizados.','Datasets de texto livre e conversas para cada tipo de treinamento.',button(ic('plus',15)+' Importar dataset','import'))+(state.datasets.length?`<div class="cards">${state.datasets.map(d=>`<article class="card"><div class="card-top"><span class="card-icon">${ic('database',19)}</span><span class="tag">${d.kind==='chat'?'CONVERSAS':'TEXTO LIVRE'}</span></div><h2>${esc(d.name)}</h2><p>${num(d.count)} exemplos · ${fixed(d.characters/1000,0)} mil caracteres</p><small class="muted">Atualizado ${date(d.updated_at||d.created_at)}</small><div class="card-actions">${button('Visualizar','preview',d.id,'small')}${button('Adicionar dados','append',d.id,'small')}<a class="btn small" href="/api/datasets/${d.id}/export">Exportar</a></div></article>`).join('')}</div>`:`<section class="panel">${empty('O conhecimento começa nos dados.','Importe JSON, JSONL, TXT, Markdown, CSV, PDF ou DOCX. Você também pode colar o conteúdo diretamente.',button(ic('plus',15)+' Importar dataset','import','','primary'))}</section>`)+`<div class="notice spacer">Conversas ensinam respostas com QLoRA. Texto livre serve para continuação de pré-treinamento ou treino do zero. Dados importados são normalizados e duplicatas exatas são removidas.</div>`}
function importForm(id=''){const d=state.datasets.find(x=>x.id===id);showModal(modalHeader(d?'Adicionar exemplos':'Importar dataset')+`<form id="dataset-form"><input type="hidden" name="dataset_id" value="${esc(id)}"><div class="field"><label for="ds-name">Nome do dataset</label><input id="ds-name" name="name" placeholder="Ex.: Assistente dos meus projetos" value="${esc(d?.name||'')}" required maxlength="100"></div><div class="dropzone spacer"><div class="dropzone-icon">${ic('upload',20)}</div><b>Selecione um ou mais arquivos</b><div class="hint">JSON · JSONL · TXT · MD · CSV · PDF · DOCX — até 32 MB por importação</div><input aria-label="Selecionar arquivos" type="file" name="files" multiple accept=".json,.jsonl,.txt,.md,.csv,.pdf,.docx"></div><div class="text-divider">OU COLE SEU CONTEÚDO</div><div class="field"><label for="ds-format">Formato do conteúdo</label><select name="format" id="ds-format"><option value="text">Texto livre — parágrafos separados por linha em branco</option><option value="json">JSON — lista de exemplos</option><option value="jsonl">JSONL — um exemplo por linha</option></select><textarea name="content" aria-label="Conteúdo do dataset" placeholder="Cole seus textos aqui…"></textarea><div class="hint" id="format-help">Cada parágrafo será um exemplo. Preserve fontes relacionadas no mesmo grupo ao preparar a avaliação.</div></div><div class="form-actions">${button('Inserir exemplo JSON','example','','small')}<button class="btn primary" type="submit">${d?'Adicionar ao dataset':'Criar dataset'}</button></div></form>`)}
function models(){return heading('O ponto de partida do seu modelo.','Adicione checkpoints do Hugging Face ou pastas locais em safetensors.',button(ic('plus',15)+' Adicionar modelo','model-form'))+`<div class="notice">O cadastro não baixa os pesos. Use “Baixar” para preparar o cache, ou deixe o download acontecer no primeiro treino. Modelos GGUF são destinados à inferência e não são aceitos aqui.</div>`+(state.models.length?`<div class="cards">${state.models.map(m=>`<article class="card"><div class="card-top"><span class="card-icon">${ic('layers',19)}</span>${badge(m.status)}</div><h2>${esc(m.name)}</h2><p class="model-repo">${esc(m.repo)}</p><p>${m.source==='hub'?'Hugging Face · '+esc(m.resolved_revision?.slice(0,8)||m.revision):'Pasta local'}</p><div class="card-actions">${m.source==='hub'?button(m.status==='ready'?'Verificar download':'Baixar modelo','download',m.id,'small'):''}<a href="#train" class="btn small">Usar em um treino ${ic('arrow',13)}</a></div></article>`).join('')}</div>`:`<section class="panel">${empty('Escolha uma base para adaptar.','Comece com um modelo pequeno de instruções. Adicione seu repositório ou use uma sugestão abaixo.')}</section>`)+`<section class="panel spacer"><h2>Comece pequeno, aprenda mais rápido.</h2><p class="muted" style="font-size:12px">Sugestões de estudo. Confira a licença de cada modelo antes de distribuir resultados.</p><div class="preset">${button('Qwen 2.5 · 1.5B Instruct','suggest','Qwen/Qwen2.5-1.5B-Instruct','small')}${button('Qwen 2.5 · 3B Instruct','suggest','Qwen/Qwen2.5-3B-Instruct','small')}${button('Qwen 2.5 · 7B Instruct','suggest','Qwen/Qwen2.5-7B-Instruct','small')}</div></section>`}
async function modelForm(){let local=await api('/local-models');showModal(modalHeader('Adicionar modelo')+`<form id="model-form"><div class="form-grid"><div class="field full"><label>Nome de exibição<input name="name" required maxlength="100" placeholder="Ex.: Qwen 1.5B Instruct"></label></div><div class="field"><label>Origem<select name="source" id="model-source"><option value="hub">Hugging Face</option><option value="local">Pasta local</option></select></label></div><div class="field"><label>Revisão ou commit<input name="revision" value="main" required maxlength="100"></label></div><div class="field full"><label>Repositório ou pasta<input name="repo" required placeholder="Qwen/Qwen2.5-1.5B-Instruct" list="local-paths"></label><datalist id="local-paths">${local.map(p=>`<option value="${esc(p)}">`).join('')}</datalist><span class="hint">Para modelo local, copie a pasta com config.json, tokenizer e pesos .safetensors para C:\AI\llm-trainning\models e informe o nome da subpasta.</span></div></div><div class="notice spacer">Repositórios privados exigem HF_TOKEN no arquivo .env e reinício do container. Código remoto de modelos permanece desativado.</div><div class="form-actions"><button class="btn primary">Adicionar modelo</button></div></form>`)}
function training(){return heading('Dê início a um novo experimento.','Escolha os dados, ajuste a receita e deixe o laboratório cuidar da execução.')+`<div class="grid-two"><section class="panel"><form id="train-form"><h2>Configure seu treinamento</h2><p class="muted" style="font-size:12px">Comece com um preset e ajuste apenas o que precisar.</p><div class="preset"><button type="button" data-action="preset" data-id="qlora">QLoRA · RTX 3090</button><button type="button" data-action="preset" data-id="scratch">Do zero · pequena LLM</button><button type="button" data-action="preset" data-id="tiny">Teste rápido · 10 passos</button></div><div class="form-grid"><div class="field full"><label>Nome do experimento<input name="name" required value="Meu primeiro treinamento" maxlength="100"></label></div><div class="field"><label>Método<select name="mode" id="train-mode"><option value="qlora">Adaptar respostas · QLoRA</option><option value="continued">Aprender texto livre · QLoRA</option><option value="scratch">Treinar do zero · LLM</option></select></label></div><div class="field"><label>Dataset<select name="dataset_id" id="train-dataset" required>${datasetOptions('qlora')}</select></label></div><div class="field full" id="base-field"><label>Modelo pré-treinado<select name="model_id" id="train-model"><option value="">Selecione um modelo</option>${state.models.map(m=>`<option value="${m.id}">${esc(m.name)}</option>`).join('')}</select></label></div><div class="field"><label>Passos de treinamento<input name="max_steps" type="number" value="100" min="1" max="1000000" required></label><span class="hint">Atualizações do otimizador. Define a duração; dados podem se repetir.</span></div><div class="field"><label>Contexto máximo<select name="context"><option>128</option><option>256</option><option>512</option><option selected>1024</option><option>2048</option><option>4096</option><option>8192</option></select></label><span class="hint">Inclui pergunta, resposta e delimitadores.</span></div></div><details><summary>Configurações avançadas</summary><div class="form-grid"><div class="field"><label>Batch físico<input type="number" name="batch" value="1" min="1" max="16"></label></div><div class="field"><label>Acumular gradientes<input type="number" name="accumulation" value="16" min="1" max="128"></label></div><div class="field"><label>Taxa de aprendizado<input type="number" name="learning_rate" value="0.0001" min="0.0000001" max="0.01" step="any"></label></div><div class="field"><label>Rank LoRA<select name="rank"><option>4</option><option>8</option><option selected>16</option><option>32</option><option>64</option></select></label></div><div class="field"><label>Fração de validação<input name="validation" type="number" value="0.1" step="0.05" min="0.05" max="0.4"></label></div><div class="field"><label>Salvar a cada N passos<input name="save_steps" type="number" value="50" min="1" max="10000"></label></div><div class="field"><label>Seed<input name="seed" type="number" value="42" min="0"></label></div><div class="field"><label>Tamanho da LLM do zero<select name="scratch_size"><option value="small">8 camadas · dimensão 512</option><option value="tiny">2 camadas · dimensão 128</option></select></label></div><div class="field full"><label class="check"><input type="checkbox" name="gradient_checkpointing" checked>Economizar VRAM com gradient checkpointing</label></div></div></details><div class="form-actions"><button class="btn primary" type="submit">${ic('zap',15)} Iniciar treinamento</button></div></form></section><div><section class="panel"><h2>Uma receita para sua bancada</h2><div class="steps"><span class="number">${ic('layers',15)}</span><div><h3>4 bits para os pesos base</h3><p>QLoRA com NF4, adaptadores LoRA e otimizador de 8 bits.</p></div></div><div class="steps"><span class="number">${ic('zap',15)}</span><div><h3>GPU utilizada diretamente</h3><p>BF16 quando suportado, TF32 e atenção SDPA do PyTorch.</p></div></div><div class="steps"><span class="number">${ic('database',15)}</span><div><h3>Dados congelados por execução</h3><p>Cópia do dataset, divisão por grupos e configuração registrada.</p></div></div></section><div class="notice">Texto livre não vira automaticamente um dataset de perguntas e respostas. Escolha o método correspondente ao seu objetivo.</div><div class="notice warn">Contextos maiores consomem mais memória. Comece com 1.024 tokens e batch 1. Os presets são pontos de partida, não garantia de capacidade para qualquer modelo.</div><p class="muted" style="font-size:11px">No treino de conversas, apenas a última resposta de cada exemplo participa da loss. Para ensinar vários turnos, forneça prefixos de conversa terminando em cada resposta desejada.</p></div></div>`}
function datasetOptions(mode){const rows=state.datasets.filter(d=>(mode==='qlora')===(d.kind==='chat'));return '<option value="">Selecione um dataset</option>'+rows.map(d=>`<option value="${d.id}">${esc(d.name)} · ${num(d.count)} exemplos</option>`).join('')}
function jobTable(jobs,paginated=false){
 if(!jobs.length)return empty('Seu primeiro experimento está por vir.','Os treinos e downloads aparecerão aqui, com status e histórico.',`<a class="btn small" href="#train">Configurar treinamento</a>`);
 const info=paginated?pagedJobs(jobs):null;
 const rows=(info?info.slice:jobs).map(runRow).join('');
 return `<div class="runs-list" role="list">${rows}</div>${paginated?pagerHtml(info):''}`;
}
function runs(){return heading('Cada experimento conta.','Histórico, checkpoints e métricas dos seus trabalhos.')+`<section class="panel runs-panel" id="jobs-table">${jobTable(state.jobs,true)}</section><div id="job-detail">${state.selected?'<p class="muted">Carregando detalhes…</p>':''}</div>`}
function chart(metrics){const points=metrics.filter(m=>typeof m.loss==='number');if(points.length<2)return `<div class="empty" style="padding:20px"><p>A curva aparece após dois registros de loss.</p></div>`;let lo=Math.min(...points.map(p=>p.loss)),hi=Math.max(...points.map(p=>p.loss));if(hi===lo)hi=lo+1;const X=i=>34+i/(points.length-1)*556,Y=v=>132-(v-lo)/(hi-lo)*104,clampY=y=>Math.min(132,Math.max(24,y));const pts=points.map((p,i)=>[X(i),Y(p.loss)]);let d=`M${pts[0][0].toFixed(1)},${pts[0][1].toFixed(1)}`;for(let i=0;i<pts.length-1;i++){const p0=pts[Math.max(0,i-1)],p1=pts[i],p2=pts[i+1],p3=pts[Math.min(pts.length-1,i+2)];d+=`C${(p1[0]+(p2[0]-p0[0])/6).toFixed(1)},${clampY(p1[1]+(p2[1]-p0[1])/6).toFixed(1)} ${(p2[0]-(p3[0]-p1[0])/6).toFixed(1)},${clampY(p2[1]-(p3[1]-p1[1])/6).toFixed(1)} ${p2[0].toFixed(1)},${p2[1].toFixed(1)}`}const last=pts.at(-1);return `<svg class="chart" viewBox="0 0 600 160" role="img" aria-label="Curva de loss de treinamento"><defs><linearGradient id="lossfill" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".2"/><stop offset="1" stop-color="var(--accent)" stop-opacity="0"/></linearGradient></defs><path d="M34 24H590M34 78H590M34 132H590" stroke="var(--line)" stroke-width="1" stroke-dasharray="2 6" fill="none"/><path d="${d}L${last[0].toFixed(1)},132L${pts[0][0].toFixed(1)},132Z" fill="url(#lossfill)"/><path d="${d}" class="chart-line" fill="none" stroke="var(--accent)" stroke-width="2.2" stroke-linejoin="round" stroke-linecap="round"/><circle cx="${last[0].toFixed(1)}" cy="${last[1].toFixed(1)}" r="3.5" fill="var(--accent)" class="chart-dot"/></svg><div class="chart-label"><span>Passo ${points[0].step}</span><span>Loss de treino ${fixed(lo,3)} a ${fixed(hi,3)}</span><span>Passo ${points.at(-1).step}</span></div>`}
async function updateDetail(){if(!state.selected||state.page!=='runs')return;const j=await api('/jobs/'+state.selected);if(state.page!=='runs'||j.id!==state.selected)return;const last=j.metrics.at(-1)||{};const loss=[...j.metrics].reverse().find(m=>typeof m.loss==='number')?.loss;const ev=[...j.metrics].reverse().find(m=>typeof m.eval_loss==='number')?.eval_loss;const pct=last.total_steps?Math.min(100,last.step/last.total_steps*100):0;parkTrainViz();$('#job-detail').innerHTML=`<section class="panel"><div class="panel-head"><div><h2>${esc(j.name)}</h2><p>${esc(j.id.slice(0,8))} · ${badge(j.status)}</p></div><div class="flex wrap">${['queued','running','cancelling'].includes(j.status)?button('Cancelar','cancel',j.id,'danger small'):''}${['cancelled','failed','interrupted'].includes(j.status)&&j.checkpoints.length?button('Retomar checkpoint','resume',j.id,'small'):''}${j.kind==='train'&&j.status==='completed'?`<a class="btn small" href="/api/jobs/${j.id}/export">${ic('download',13)} Exportar artefato</a>`:''}</div></div>${j.error?`<div class="error-inline">${esc(j.error)}</div>`:''}${j.kind==='train'?`<div class="metrics"><div class="metric"><small>Passos</small><b>${last.step||0} / ${last.total_steps||j.config.max_steps}</b></div><div class="metric"><small>Loss de treino</small><b>${fixed(loss,4)}</b></div><div class="metric"><small>Loss de validação</small><b>${fixed(ev,4)}</b></div></div><div class="progress"><span style="width:${pct}%"></span></div><div id="train-viz-host" class="train-viz-host"></div>${chart(j.metrics)}<div class="system-grid"><span>Pico alocado <b>${fixed(last.vram_gb)} GiB</b></span><span>Checkpoints <b>${j.checkpoints.length}</b></span><span>Tokens treino <b>${num(j.training_info?.token_counts?.train)}</b></span>${['running','cancelling','queued'].includes(j.status)?`<span>GPU agora <b>${state.system?.gpu?fixed(state.system.gpu.utilization,0)+'%':'—'}</b></span><span>VRAM <b>${state.system?.gpu?fixed(state.system.gpu.used_mb/1024,1)+' / '+fixed(state.system.gpu.total_mb/1024,0)+' GB':'—'}</b></span><span>RAM <b>${state.system?fixed(state.system.ram_used/2**30,0)+' / '+fixed(state.system.ram_total/2**30,0)+' GB':'—'}</b></span><span>CPU <b>${state.system?fixed(state.system.cpu_percent,0)+'%':'—'}</b></span>`:''}</div>`:''}${j.kind==='generate'&&j.result?`<div class="result-text">${esc(j.result.text)}</div><p class="hint">${j.result.tokens} tokens · ${fixed(j.result.seconds)} s</p>`:''}<details ${j.status==='failed'?'open':''}><summary>Logs do processo</summary><pre class="logs">${esc(j.log||'Aguardando execução…')}</pre></details><details><summary>Configuração registrada</summary><pre class="preview">${esc(JSON.stringify(j.config,null,2))}</pre></details></section>`;bindTrainViz(j)}
function playground(){
 const trained=state.jobs.filter(j=>j.kind==='train'&&j.status==='completed');
 const pg=state.pg||{};
 const opt=(val,label)=>`<option value="${esc(val)}" ${pg.target===val?'selected':''}>${esc(label)}</option>`;
 const busy=generating();
 return heading('Converse com o resultado.','Teste um modelo base ou um treinamento concluído. A conversa fica nesta sessão.')+`
 <form id="generate-form" class="pg-layout">
  <div class="pg-stage">
   <div class="pg-toolbar"><button type="button" class="btn small" data-action="new-chat">Nova conversa</button></div>
   <div id="generation-result" class="pg-thread" role="log" aria-live="polite" aria-relevant="additions">${chatThreadHtml()}</div>
   <div class="pg-composer">
    <label class="sr-only" for="pg-prompt">Mensagem</label>
    <textarea id="pg-prompt" name="prompt" rows="1" maxlength="16000" placeholder="Pergunte qualquer coisa"></textarea>
    <button type="button" class="btn danger small" id="pg-cancel" data-action="cancel" data-id="${esc(state.generation||'')}" ${busy?'':'hidden'}>Cancelar</button>
    <button type="submit" class="btn primary pg-send" id="pg-send" aria-label="Enviar mensagem" ${busy?'hidden':''}>${ic('send',16)}</button>
   </div>
   <p class="notice pg-note">A LLM treinada do zero completa texto: ele pode ainda não saber conversar. Modelos adaptados usam o template da base. A instrução de sistema é ignorada em modelos sem formato de chat.</p>
  </div>
  <aside class="pg-rail" aria-label="Configurações da conversa">
   <h2>Modelo</h2>
   <div class="field"><label for="pg-target">Origem</label>
    <select name="target" id="pg-target" required>
     <option value="">Selecione um modelo</option>
     <optgroup label="Modelos base">${state.models.map(m=>opt('model:'+m.id,m.name)).join('')}</optgroup>
     <optgroup label="Seus treinamentos">${trained.map(j=>opt('run:'+j.id,j.name)).join('')}</optgroup>
    </select>
   </div>
   <div class="field"><label for="pg-system">Instrução de sistema</label>
    <textarea id="pg-system" name="system" maxlength="4000">${esc(pg.system||'Responda em português de forma clara.')}</textarea>
   </div>
   <div class="field"><label for="pg-tokens">Máximo de novos tokens</label>
    <input id="pg-tokens" name="max_tokens" type="number" min="1" max="1024" value="${esc(pg.max_tokens||200)}">
   </div>
   <div class="field"><label for="pg-temp">Temperatura</label>
    <input id="pg-temp" name="temperature" type="number" min="0" max="2" step="0.1" value="${esc(pg.temperature??0)}">
    <span class="hint">Zero para comparação determinística.</span>
   </div>
  </aside>
 </form>`;
}
async function updateGeneration(){
 if(state.page!=='playground')return;
 const pending=state.chat.filter(m=>m.jobId&&['queued','running','cancelling','preparing'].includes(m.status));
 if(!pending.length){syncComposer();return}
 for(const msg of pending){
  try{
   const j=await api('/jobs/'+msg.jobId);
   if(state.page!=='playground')return;
   msg.status=j.status;
   if(j.result){
    msg.text=j.result.text||'(O modelo gerou apenas tokens especiais.)';
    msg.meta=`${j.result.tokens} tokens · ${fixed(j.result.seconds)} s · ${fixed(j.result.peak_vram_gb)} GiB`;
    msg.status='completed';
   }else if(j.error&&['failed','cancelled'].includes(j.status)){
    msg.text=j.error;
   }
  }catch(e){msg.status='failed';msg.text=e.message}
 }
 paintChat();
 syncComposer();
}
function changeMode(){const f=$('#train-form');if(!f)return;const mode=f.elements.mode.value;$('#train-dataset').innerHTML=datasetOptions(mode);const fromBase=!['scratch','scratch_moe'].includes(mode);$('#base-field').hidden=!fromBase;$('#train-model').required=fromBase;$('#moe-fields').hidden=mode!=='scratch_moe';}
document.addEventListener('change',event=>{
 if(event.target.id==='train-mode')changeMode();
 if(['pg-target','pg-system','pg-tokens','pg-temp'].includes(event.target.id)){rememberPg();syncComposer()}
});
document.addEventListener('click',async event=>{const b=event.target.closest('[data-action]');if(!b)return;const {action,id}=b.dataset;try{
 if(action==='close')$('#modal').close();
 else if(action==='import')importForm();
 else if(action==='append')importForm(id);
 else if(action==='example'){const f=$('#dataset-form');f.elements.format.value='json';f.elements.content.value=JSON.stringify([{instruction:'O que é um checkpoint?',output:'É um registro do estado do treinamento que permite guardar ou retomar o trabalho.',group:'conceito_checkpoint'},{instruction:'Qual é a função do dataset?',output:'Fornecer exemplos para o modelo aprender padrões.',group:'conceito_dataset'},{instruction:'O que é uma época?',output:'Uma passagem pelo conjunto de treinamento.',group:'conceito_epoca'}],null,2);}
 else if(action==='preview'){const d=await api('/datasets/'+id);showModal(modalHeader(esc(d.name))+`<p class="hint">Prévia de até 20 exemplos · ${num(d.count)} no total.</p><pre class="preview">${esc(JSON.stringify(d.examples,null,2))}</pre>`)}
 else if(action==='model-form')await modelForm();
 else if(action==='suggest'){await post('/models',{name:id.split('/')[1],repo:id});await refresh();render();toast('Modelo cadastrado. Você pode baixá-lo agora.');}
 else if(action==='download'){b.disabled=true;const j=await post('/models/'+id+'/download');state.selected=j.id;await refresh();location.hash='runs';if(state.page==='runs')render();toast('Download adicionado à fila.');}
 else if(action==='job'){state.selected=id;if(state.page==='runs'){const el=$('#jobs-table');if(el)el.innerHTML=jobTable(state.jobs,true);await updateDetail()}else location.hash='runs';}
 else if(action==='runs-page'){const n=Number(id);if(n>=1){state.runsPage=n;const el=$('#jobs-table');if(el)el.innerHTML=jobTable(state.jobs,true)}}
 else if(action==='runs-size'){try{localStorage.setItem('ows.pageSize',id)}catch(e){}state.runsPage=1;const el=$('#jobs-table');if(el)el.innerHTML=jobTable(state.jobs,true)}
 else if(action==='new-chat'){state.chat=[];state.generation=null;paintChat();syncComposer();const ta=$('#pg-prompt');if(ta)ta.focus()}
 else if(action==='cancel'){await post('/jobs/'+id+'/cancel');toast('Cancelamento solicitado. Aguarde a liberação do processo.');await refresh();if(state.page==='runs')await updateDetail();if(state.page==='playground')await updateGeneration();}
 else if(action==='resume'){await post('/jobs/'+id+'/resume');toast('Retomada adicionada à fila.');await refresh();await updateDetail();}
 else if(action==='preset'){const f=$('#train-form');const preset=id==='qlora'?{mode:'qlora',context:1024,batch:1,accumulation:16,learning_rate:0.0001,max_steps:100,save_steps:50,scratch_size:'small'}:id==='scratch'?{mode:'scratch',context:512,batch:4,accumulation:8,learning_rate:0.0003,max_steps:1000,save_steps:100,scratch_size:'small'}:{mode:'scratch',context:128,batch:2,accumulation:1,learning_rate:0.0003,max_steps:10,save_steps:5,scratch_size:'tiny'};Object.entries(preset).forEach(([k,v])=>f.elements[k].value=v);changeMode();toast('Preset aplicado. Escolha seu dataset.');}
 }catch(e){toast(e.message,true)}finally{b.disabled=false}});
document.addEventListener('submit',async event=>{const form=event.target;if(!['dataset-form','model-form','train-form','generate-form','reset-form'].includes(form.id))return;event.preventDefault();const submit=form.querySelector('[type=submit]')||form.querySelector('button:last-child');submit.disabled=true;try{
 if(form.id==='reset-form'){if(form.elements.confirmation.value!=='RESET_WORKSPACE')throw new Error('Digite RESET_WORKSPACE exatamente.');if(!confirm('Apagar todos os dados do workspace?'))return;await api('/reset',{method:'POST',headers:{'x-reset-confirmation':'RESET_WORKSPACE'}});await refresh();location.hash='overview';toast('Workspace limpo.');}
 else if(form.id==='dataset-form'){const fd=new FormData(form);if(!form.elements.files.files.length)fd.delete('files');const d=await api('/datasets/import',{method:'POST',body:fd});$('#modal').close();await refresh();render();toast(`${d.count} exemplos no dataset. ${d.last_duplicates} duplicata(s) removida(s).`)}
 else if(form.id==='model-form'){await post('/models',Object.fromEntries(new FormData(form)));$('#modal').close();await refresh();render();toast('Modelo adicionado.');}
 else if(form.id==='train-form'){const data=Object.fromEntries(new FormData(form));for(const k of ['context','batch','accumulation','max_steps','learning_rate','rank','validation','seed','save_steps'])data[k]=Number(data[k]);data.gradient_checkpointing=form.elements.gradient_checkpointing.checked;data.model_id=data.mode==='scratch'?null:data.model_id;const j=await post('/train',data);state.selected=j.id;await refresh();location.hash='runs';toast('Treinamento adicionado à fila.');}
 else if(form.id==='generate-form'){rememberPg();const data=Object.fromEntries(new FormData(form));const prompt=(data.prompt||'').trim();if(!data.target||!prompt)return;const [type,id]=data.target.split(':');delete data.target;data.prompt=prompt;data[type==='run'?'run_id':'model_id']=id;data.max_tokens=Number(data.max_tokens);data.temperature=Number(data.temperature);state.chat.push({role:'user',text:prompt,status:'done'});form.elements.prompt.value='';const ta=$('#pg-prompt');if(ta){ta.style.height='auto'}paintChat();syncComposer();try{const j=await post('/generate',data);state.generation=j.id;state.chat.push({role:'assistant',text:'',jobId:j.id,status:'queued'});paintChat();syncComposer()}catch(err){state.chat.push({role:'assistant',text:err.message,status:'failed'});paintChat();syncComposer();throw err}}
 }catch(e){toast(e.message,true)}finally{submit.disabled=false;if(form.id==='generate-form')syncComposer()}});
window.addEventListener('hashchange',route);
document.addEventListener('input',event=>{
 if(event.target.id==='pg-prompt'){
  event.target.style.height='auto';
  event.target.style.height=Math.min(event.target.scrollHeight,200)+'px';
  syncComposer();
 }
});
document.addEventListener('keydown',event=>{
 if(event.target.id==='pg-prompt'&&event.key==='Enter'&&!event.shiftKey){
  event.preventDefault();
  const send=$('#pg-send');
  if(send&&!send.disabled&&!send.hidden){const f=$('#generate-form');if(f)f.requestSubmit()}
 }
 if(event.target.classList.contains('run-row')&&(event.key==='Enter'||event.key===' ')){
  event.preventDefault();
  event.target.click();
 }
});
let polling=false;
async function poll(){if(polling)return;polling=true;try{await refresh();if(state.page==='overview')render();else if(state.page==='runs'){const el=$('#jobs-table');if(el)el.innerHTML=jobTable(state.jobs,true);await updateDetail()}else if(state.page==='playground')await updateGeneration();else if(state.page==='models'&&!$('#modal').open)render();}catch(e){const conn=$('#connection');conn.className='connection down';conn.innerHTML='<i></i> Sem conexão com o container';}finally{polling=false}}
(async()=>{try{await refresh();route();}catch(e){$('#main').innerHTML=`<div class="error-inline">${esc(e.message)}</div>`;toast('Não foi possível conectar à aplicação.',true)}setInterval(poll,3000)})();

// MoE controls are kept in this small override so the original UI remains easy
// to read while the form can evolve without a framework build step.
function training(){return heading('Dê início a um novo experimento.','Escolha os dados, ajuste a receita e deixe o laboratório cuidar da execução.')+`<div class="grid-two"><section class="panel"><form id="train-form"><h2>Configure seu treinamento</h2><p class="muted" style="font-size:12px">Comece com um preset e ajuste apenas o que precisar.</p><div class="preset"><button type="button" data-action="preset" data-id="qlora">QLoRA · RTX 3090</button><button type="button" data-action="preset" data-id="scratch">Do zero · pequena LLM</button><button type="button" data-action="preset" data-id="moe">MoE · 4 experts</button><button type="button" data-action="preset" data-id="tiny">Teste rápido · 10 passos</button></div><div class="form-grid"><div class="field full"><label>Nome do experimento<input name="name" required value="Meu primeiro treinamento" maxlength="100"></label></div><div class="field"><label>Método<select name="mode" id="train-mode"><option value="qlora">Adaptar respostas · QLoRA</option><option value="continued">Aprender texto livre · QLoRA</option><option value="scratch">Treinar do zero · LLM</option><option value="scratch_moe">Treinar do zero · MoE esparso</option></select></label></div><div class="field"><label>Dataset<select name="dataset_id" id="train-dataset" required>${datasetOptions('qlora')}</select></label></div><div class="field full" id="base-field"><label>Modelo pré-treinado<select name="model_id" id="train-model"><option value="">Selecione um modelo</option>${state.models.map(m=>`<option value="${m.id}">${esc(m.name)}</option>`).join('')}</select></label></div><div class="field"><label>Passos de treinamento<input name="max_steps" type="number" value="100" min="1" max="1000000" required></label><span class="hint">Atualizações do otimizador. Define a duração; dados podem se repetir.</span></div><div class="field"><label>Contexto máximo<select name="context"><option>128</option><option>256</option><option>512</option><option selected>1024</option><option>2048</option><option>4096</option><option>8192</option></select></label><span class="hint">Comece com 256–512 no MoE para caber confortavelmente na 3090.</span></div></div><details><summary>Configurações avançadas</summary><div class="form-grid"><div class="field"><label>Batch físico<input type="number" name="batch" value="1" min="1" max="16"></label></div><div class="field"><label>Acumular gradientes<input type="number" name="accumulation" value="16" min="1" max="128"></label></div><div class="field"><label>Taxa de aprendizado<input type="number" name="learning_rate" value="0.0001" min="0.0000001" max="0.01" step="any"></label></div><div class="field"><label>Rank LoRA<select name="rank"><option>4</option><option>8</option><option selected>16</option><option>32</option><option>64</option></select></label></div><div class="field"><label>Fração de validação<input name="validation" type="number" value="0.1" step="0.05" min="0.05" max="0.4"></label></div><div class="field"><label>Salvar a cada N passos<input name="save_steps" type="number" value="50" min="1" max="10000"></label></div><div class="field"><label>Seed<input name="seed" type="number" value="42" min="0"></label></div><div class="field"><label>Tamanho da LLM do zero<select name="scratch_size"><option value="small">LLM: 8 camadas · dimensão 512</option><option value="tiny">Pequeno: 2 camadas · dimensão 128</option></select></label></div><div id="moe-fields" class="field full" hidden><div class="notice"><b>Roteamento MoE</b><div class="form-grid spacer"><div class="field"><label>Experts<input type="number" name="num_experts" value="4" min="2" max="16"><span class="hint">Especialistas feed-forward.</span></label></div><div class="field"><label>Top-k por token<input type="number" name="top_k" value="2" min="1" max="4"><span class="hint">2 é o preset recomendado.</span></label></div><div class="field"><label>Fator de capacidade<input type="number" name="capacity_factor" value="1.25" min="1" max="2" step="0.05"></label></div><div class="field"><label>Penalidade de balanceamento<input type="number" name="router_aux_loss_coef" value="0.01" min="0" max="1" step="0.001"></label></div></div><small>O router escolhe os melhores experts e a penalidade evita que todos os tokens caiam no mesmo especialista.</small></div></div><div class="field full"><label class="check"><input type="checkbox" name="gradient_checkpointing" checked>Economizar VRAM com gradient checkpointing</label></div></div></details><div class="form-actions"><button class="btn primary" type="submit">${ic('zap',15)} Iniciar treinamento</button></div></form></section><div><section class="panel"><h2>Uma receita para sua bancada</h2><div class="steps"><span class="number">${ic('layers',15)}</span><div><h3>4 bits para os pesos base</h3><p>QLoRA com NF4, adaptadores LoRA e otimizador de 8 bits.</p></div></div><div class="steps"><span class="number">${ic('zap',15)}</span><div><h3>GPU utilizada diretamente</h3><p>BF16 quando suportado, TF32 e atenção SDPA do PyTorch.</p></div></div><div class="steps"><span class="number">${ic('runs',15)}</span><div><h3>MoE experimental local</h3><p>O modo MoE usa top-k routing e experts pequenos, ideal para aprender a arquitetura em uma GPU.</p></div></div><div class="steps"><span class="number">${ic('database',15)}</span><div><h3>Dados congelados por execução</h3><p>Cópia do dataset, divisão por grupos e configuração registrada.</p></div></div></section><div class="notice">Texto livre não vira automaticamente um dataset de perguntas e respostas. Escolha o método correspondente ao seu objetivo.</div><div class="notice warn">MoE aumenta a capacidade total com especialistas, mas este preset é educacional e single-GPU. Modelos MoE de produção exigem paralelismo distribuído e mais memória.</div><p class="muted" style="font-size:11px">No treino de conversas, apenas a última resposta de cada exemplo participa da loss.</p></div></div>`}
document.addEventListener('click',event=>{const b=event.target.closest('[data-action="preset"][data-id="moe"]');if(!b)return;const f=$('#train-form');if(!f)return;const preset={mode:'scratch_moe',context:256,batch:1,accumulation:8,learning_rate:0.0003,max_steps:100,save_steps:25,scratch_size:'tiny',num_experts:4,top_k:2,capacity_factor:1.25,router_aux_loss_coef:0.01};Object.entries(preset).forEach(([k,v])=>{if(f.elements[k])f.elements[k].value=v});changeMode();toast('Preset MoE aplicado. Escolha um dataset de texto livre.');});
const renderRunDetail = updateDetail;
updateDetail = async function(){await renderRunDetail();const panel=$('#job-detail .panel');if(!panel||$('#moe-summary'))return;const job=await api('/jobs/'+state.selected);if(job.training_info?.architecture!=='moe')return;const moe=job.training_info.moe||{};const counts=job.metrics?.at(-1)?.expert_counts||[];panel.insertAdjacentHTML('beforeend',`<div id="moe-summary" class="notice spacer"><b>Roteamento MoE</b><p>${moe.num_experts||'—'} experts · top-${moe.top_k||'—'} por token · fator ${moe.capacity_factor||'—'}</p>${counts.length?`<small>Uso no último registro: ${counts.map((v,i)=>`E${i+1} ${fixed(v*100,1)}%`).join(' · ')}</small>`:''}</div>`)};

// GGUF export is attached to completed runs so the training artifact remains
// reusable, while each quantization gets its own durable queue job.
const renderRunDetailWithMoe = updateDetail;
updateDetail = async function(){
 await renderRunDetailWithMoe();
 const panel=$('#job-detail .panel');
 if(!panel||!state.selected||$('#gguf-controls'))return;
 const job=await api('/jobs/'+state.selected);
 if(job.kind==='gguf'){
  if(job.status==='completed'&&job.artifact){panel.insertAdjacentHTML('beforeend',`<div id="gguf-controls" class="notice spacer"><b>Arquivo GGUF pronto</b><p>${esc(job.config?.quantization||'')} · ${fixed((job.result?.size_bytes||0)/2**20,1)} MiB · SHA-256 <code>${esc(job.result?.sha256||'')}</code></p><a class="btn small" href="/api/jobs/${job.id}/download">${ic('download',13)} Baixar GGUF</a></div>`)}
  return;
 }
 if(job.kind!=='train'||job.status!=='completed')return;
 const compatible=['qlora','continued'].includes(job.config?.mode);
 if(!compatible){const reason=job.config?.mode==='scratch'?'A LLM do zero usa um tokenizer BPE próprio que o llama.cpp não reconhece.':'O MoE customizado ainda não possui mapeamento oficial no llama.cpp.';panel.insertAdjacentHTML('beforeend',`<div id="gguf-controls" class="notice warn spacer"><b>Exportação GGUF</b><p>${reason} Preserve o artefato safetensors para continuar os estudos.</p></div>`);return;}
 const exports=state.jobs.filter(x=>x.kind==='gguf'&&x.config?.run_id===job.id);
 const rows=exports.map(x=>x.status==='completed'&&x.artifact?`<div class="flex wrap"><a class="btn small" href="/api/jobs/${x.id}/download">${ic('download',13)} ${esc(x.config.quantization)} · ${fixed((x.result?.size_bytes||0)/2**20,1)} MiB</a><small class="muted">SHA ${esc((x.result?.sha256||'').slice(0,12))}…</small></div>`:`<div class="flex">${badge(x.status)}<span>${esc(x.config?.quantization||'')} <small class="muted">${esc(x.error||'')}</small></span></div>`).join('');
 panel.insertAdjacentHTML('beforeend',`<div id="gguf-controls" class="notice spacer"><b>Exportar para OpenWeights</b><p>Mescla o adaptador quando necessário, converte o checkpoint e gera um arquivo GGUF para inferência local.</p><div class="preset"><button type="button" data-action="gguf" data-id="${job.id}" data-quant="Q4_K_M">Q4_K_M · menor</button><button type="button" data-action="gguf" data-id="${job.id}" data-quant="Q5_K_M">Q5_K_M · equilibrado</button><button type="button" data-action="gguf" data-id="${job.id}" data-quant="Q8_0">Q8_0 · qualidade</button><button type="button" data-action="gguf" data-id="${job.id}" data-quant="F16">F16 · sem quantizar</button></div>${rows?`<div class="steps"><span class="number">${ic('download',14)}</span><div><h3>Exportações deste treinamento</h3><div class="flex" style="flex-direction:column;align-items:flex-start;gap:8px">${rows}</div></div></div>`:''}<small>O merge e a conversão usam CPU, RAM e disco temporário. A fila mantém uma exportação por vez para não disputar a GPU com um treino.</small></div>`);
};
document.addEventListener('click',async event=>{
 const b=event.target.closest('[data-action="gguf"]');
 if(!b)return;
 b.disabled=true;
 try{const sourceId=b.dataset.id;await post('/jobs/'+sourceId+'/gguf',{quantization:b.dataset.quant});await refresh();state.selected=sourceId;await updateDetail();toast('Exportação GGUF adicionada à fila.');}
 catch(e){toast(e.message,true)}finally{b.disabled=false}
});

function loadThree(){
 if(window.THREE)return Promise.resolve(window.THREE);
 if(window.__owsThreeP)return window.__owsThreeP;
 window.__owsThreeP=new Promise((resolve,reject)=>{
  const s=document.createElement('script');
  s.src='https://cdn.jsdelivr.net/npm/three@0.160.0/build/three.min.js';
  s.async=true;
  s.onload=()=>window.THREE?resolve(window.THREE):reject(new Error('THREE'));
  s.onerror=()=>reject(new Error('cdn'));
  document.head.appendChild(s);
 });
 return window.__owsThreeP;
}
function vizHudHtml(job,m){
 m=m||{};
 return `<span>Passo <b>${num(m.step||0)} / ${num(m.total||job.config?.max_steps||0)}</b></span><span>Loss <b>${fixed(m.loss,4)}</b></span><span>VRAM <b>${m.vram==null?'—':fixed(m.vram)+' GiB'}</b></span><span>${esc(statusText[m.status]||m.status||'')}</span>`;
}
function vizFallbackMarkup(moe){
 const mid=moe
  ?`<span class="tv2d-node"></span><span class="tv2d-wire"></span><span class="tv2d-node is-attn"></span><span class="tv2d-wire"></span><span class="tv2d-node is-expert"></span><span class="tv2d-wire"></span><span class="tv2d-node is-expert"></span><span class="tv2d-wire"></span><span class="tv2d-node is-expert"></span>`
  :`<span class="tv2d-node"></span><span class="tv2d-wire"></span><span class="tv2d-node is-attn"></span><span class="tv2d-wire"></span><span class="tv2d-node is-ffn"></span><span class="tv2d-wire"></span><span class="tv2d-node is-attn"></span><span class="tv2d-wire"></span><span class="tv2d-node is-ffn"></span>`;
 return `<div class="tv2d" aria-hidden="true"><i class="tv2d-token"></i><i class="tv2d-token"></i><i class="tv2d-token"></i><i class="tv2d-token is-back"></i><i class="tv2d-token is-back"></i><div class="tv2d-flow">${mid}</div></div>`;
}
function parkTrainViz(){
 const v=window.__owsViz;
 if(!v||!v.wrap)return;
 if(!v._park)v._park=document.createDocumentFragment();
 if(v.wrap.parentNode)v._park.appendChild(v.wrap);
}
function disposeTrainViz(){
 const v=window.__owsViz;
 if(!v)return;
 v.alive=false;
 if(v.raf){cancelAnimationFrame(v.raf);v.raf=0}
 if(v.ro)try{v.ro.disconnect()}catch(e){}
 if(v.disposables)v.disposables.forEach(d=>{try{if(d&&d.dispose)d.dispose()}catch(e){}});
 if(v.renderer){
  try{v.renderer.dispose()}catch(e){}
  try{if(v.renderer.forceContextLoss)v.renderer.forceContextLoss()}catch(e){}
  const gl=v.renderer.getContext&&v.renderer.getContext();
  if(gl&&gl.getExtension){const ext=gl.getExtension('WEBGL_lose_context');if(ext)ext.loseContext()}
 }
 if(v.wrap&&v.wrap.parentNode)v.wrap.remove();
 window.__owsViz=null;
}
function bindTrainViz(job){
 if(!trainVizActive(job)){disposeTrainViz();return}
 const host=$('#train-viz-host');
 if(!host)return;
 const v=window.__owsViz;
 if(v&&v.jobId===job.id&&v.wrap&&(v.renderer||v.loading||v.fallback)){
  if(!host.contains(v.wrap))host.appendChild(v.wrap);
  if(v.resize)v.resize();
  updateTrainViz(job);
  return;
 }
 disposeTrainViz();
 mountTrainViz(host,job);
}
function updateTrainViz(job){
 const v=window.__owsViz;
 if(!v||!v.alive)return;
 const last=(job.metrics&&job.metrics.at(-1))||{};
 const loss=[...(job.metrics||[])].reverse().find(m=>typeof m.loss==='number')?.loss;
 v.metrics={step:last.step||0,total:last.total_steps||job.config?.max_steps||0,loss,vram:last.vram_gb,expert_counts:last.expert_counts,status:job.status};
 if(typeof loss==='number'){if(v.lossMax==null||loss>v.lossMax)v.lossMax=loss}
 const hud=v.wrap&&v.wrap.querySelector('.train-viz-hud');
 if(hud)hud.innerHTML=vizHudHtml(job,v.metrics);
 if(v.experts&&Array.isArray(v.metrics.expert_counts)){
  v.metrics.expert_counts.forEach((c,i)=>{if(v.experts[i]&&v.experts[i].material)v.experts[i].material.emissiveIntensity=0.18+Math.min(1,c)*1.1});
 }
 if(v.lossMesh&&v.lossMax){const t=1-(v.metrics.loss??v.lossMax)/v.lossMax;v.lossMesh.position.y=1.5-Math.max(0,Math.min(1,t))*3.1}
 if(v.reduced&&v.renderer&&v.scene)v.renderer.render(v.scene,v.camera);
}
async function mountTrainViz(host,job){
 const wrap=document.createElement('div');
 wrap.className='train-viz';
 wrap.setAttribute('role','img');
 wrap.setAttribute('aria-label','Visualização do fluxo de treinamento');
 wrap.innerHTML=`<div class="train-viz-hud">${vizHudHtml(job,{})}</div><canvas class="train-viz-canvas" aria-hidden="true"></canvas><div class="train-viz-fallback" hidden>${vizFallbackMarkup(isMoeJob(job))}</div>`;
 host.appendChild(wrap);
 const viz={jobId:job.id,wrap,alive:true,loading:true,moe:isMoeJob(job),disposables:[],reduced:window.matchMedia('(prefers-reduced-motion: reduce)').matches};
 window.__owsViz=viz;
 try{
  await loadThree();
  if(!viz.alive||window.__owsViz!==viz)return;
  initTrainScene(viz,job);
  viz.loading=false;
  updateTrainViz(job);
 }catch(e){
  if(!viz.alive||window.__owsViz!==viz)return;
  viz.loading=false;
  viz.fallback=true;
  const canvas=wrap.querySelector('canvas');
  const fb=wrap.querySelector('.train-viz-fallback');
  if(canvas)canvas.hidden=true;
  if(fb)fb.hidden=false;
  updateTrainViz(job);
 }
}
function initTrainScene(viz,job){
 const THREE=window.THREE;
 const canvas=viz.wrap.querySelector('canvas');
 const renderer=new THREE.WebGLRenderer({canvas,antialias:true,alpha:false,powerPreference:'low-power'});
 renderer.setPixelRatio(Math.min(2,window.devicePixelRatio||1));
 renderer.setClearColor(0x0a0b0d,1);
 const scene=new THREE.Scene();
 scene.fog=new THREE.Fog(0x0a0b0d,10,26);
 const camera=new THREE.PerspectiveCamera(40,2,0.1,80);
 camera.position.set(0,3.6,11.2);
 camera.lookAt(0,0.2,0);
 const bag=viz.disposables;
 const track=obj=>{if(obj.geometry)bag.push(obj.geometry);const mats=obj.material?(Array.isArray(obj.material)?obj.material:[obj.material]):[];mats.forEach(m=>bag.push(m));return obj};
 const amb=new THREE.AmbientLight(0x3d342c,0.85);scene.add(amb);
 const key=new THREE.PointLight(0xff8f1f,1.35,42);key.position.set(5,6.2,6);scene.add(key);
 const fill=new THREE.PointLight(0xffb54d,0.32,28);fill.position.set(-6,-1.4,4);scene.add(fill);
 const floor=new THREE.Mesh(new THREE.PlaneGeometry(22,12),new THREE.MeshBasicMaterial({color:0x0e1013}));
 floor.rotation.x=-Math.PI/2;floor.position.y=-2.2;scene.add(track(floor));
 const grid=new THREE.GridHelper(18,18,0x1e222a,0x1e222a);grid.position.y=-2.18;scene.add(track(grid));
 const lineMat=new THREE.LineBasicMaterial({color:0x1e222a});bag.push(lineMat);
 const ringGeo=new THREE.TorusGeometry(0.72,0.045,8,40);bag.push(ringGeo);
 const boxAttn=new THREE.BoxGeometry(0.7,1.15,0.7);bag.push(boxAttn);
 const boxFfn=new THREE.BoxGeometry(0.55,1.55,0.55);bag.push(boxFfn);
 const matAttn=new THREE.MeshStandardMaterial({color:0x181b21,emissive:0xff8f1f,emissiveIntensity:0.28,roughness:0.45,metalness:0.22});bag.push(matAttn);
 const matFfn=new THREE.MeshStandardMaterial({color:0x121418,emissive:0xffb54d,emissiveIntensity:0.16,roughness:0.5,metalness:0.18});bag.push(matFfn);
 const matRing=new THREE.MeshBasicMaterial({color:0xff8f1f,transparent:true,opacity:0.55});bag.push(matRing);
 const sphere=new THREE.SphereGeometry(0.11,10,10);bag.push(sphere);
 const fwdMat=new THREE.MeshStandardMaterial({color:0xff8f1f,emissive:0xff8f1f,emissiveIntensity:0.7,roughness:0.35,metalness:0.15});bag.push(fwdMat);
 const backMat=new THREE.MeshStandardMaterial({color:0xf25c05,emissive:0xf25c05,emissiveIntensity:0.25,roughness:0.6,transparent:true,opacity:0.38});bag.push(backMat);
 const fwd=[],back=[],experts=[],anchors=[];
 function addLine(a,b){
  const g=new THREE.BufferGeometry().setFromPoints([a,b]);bag.push(g);
  scene.add(new THREE.Line(g,lineMat));
 }
 if(viz.moe){
  const n=Math.max(2,Math.min(8,Number(job.training_info?.moe?.num_experts||job.config?.num_experts||4)));
  const routerGeo=new THREE.OctahedronGeometry(0.62);bag.push(routerGeo);
  const routerMat=new THREE.MeshStandardMaterial({color:0x181b21,emissive:0xffb54d,emissiveIntensity:0.55,roughness:0.35,metalness:0.3});bag.push(routerMat);
  const router=new THREE.Mesh(routerGeo,routerMat);router.position.set(-0.4,0.2,0);scene.add(router);
  const inMesh=new THREE.Mesh(boxAttn,matAttn);inMesh.position.set(-5.2,0.15,0);scene.add(inMesh);
  const outMesh=new THREE.Mesh(boxFfn,matFfn);outMesh.position.set(5.4,0.15,0);scene.add(outMesh);
  addLine(new THREE.Vector3(-5.2,0.15,0),new THREE.Vector3(-0.4,0.2,0));
  for(let i=0;i<n;i++){
   const a=(-0.7+i/(n-1||1)*1.4);
   const mesh=new THREE.Mesh(boxFfn,matFfn.clone());bag.push(mesh.material);
   mesh.position.set(2.3,Math.sin(a)*1.7,Math.cos(a)*1.35);
   scene.add(mesh);experts.push(mesh);
   addLine(new THREE.Vector3(-0.4,0.2,0),mesh.position.clone());
   addLine(mesh.position.clone(),new THREE.Vector3(5.4,0.15,0));
  }
  viz.experts=experts;
  anchors.push(inMesh.position.clone(),router.position.clone(),outMesh.position.clone());
 }else{
  const xs=[-5.4,-3.2,-1.05,1.05,3.2,5.4];
  xs.forEach((x,i)=>{
   const attn=i%2===0;
   const mesh=new THREE.Mesh(attn?boxAttn:boxFfn,attn?matAttn:matFfn);
   mesh.position.set(x,0.1,0);scene.add(mesh);
   const ring=new THREE.Mesh(ringGeo,matRing);
   ring.rotation.y=Math.PI/2;ring.position.set(x,0.1,0);scene.add(ring);
   anchors.push(mesh.position.clone());
   if(i)addLine(new THREE.Vector3(xs[i-1],0.1,0),new THREE.Vector3(x,0.1,0));
  });
 }
 const lossGeo=new THREE.SphereGeometry(0.16,12,12);bag.push(lossGeo);
 const lossMat=new THREE.MeshStandardMaterial({color:0xffb54d,emissive:0xff8f1f,emissiveIntensity:0.9,roughness:0.3});bag.push(lossMat);
 const lossMesh=new THREE.Mesh(lossGeo,lossMat);lossMesh.position.set(6.6,1.5,0.8);scene.add(lossMesh);
 viz.lossMesh=lossMesh;
 for(let i=0;i<10;i++){const m=new THREE.Mesh(sphere,fwdMat);scene.add(m);fwd.push(m)}
 for(let i=0;i<7;i++){const m=new THREE.Mesh(sphere,backMat);scene.add(m);back.push(m)}
 viz.fwd=fwd;viz.back=back;
 function place(mesh,u,lane){
  const x=-6.4+u*12.8;
  const y=(lane>0?0.42:-0.55)+Math.sin(u*Math.PI*6+lane)*0.28;
  const z=Math.cos(u*Math.PI*3+lane)*0.22;
  mesh.position.set(x,y,z);
 }
 function resize(){
  const w=viz.wrap.clientWidth||640;
  const h=Math.max(260,Math.min(360,viz.wrap.clientHeight||320));
  renderer.setSize(w,h,false);
  camera.aspect=w/h;camera.updateProjectionMatrix();
 }
 const ro=new ResizeObserver(resize);ro.observe(viz.wrap);viz.ro=ro;viz.resize=resize;resize();
 viz.renderer=renderer;viz.scene=scene;viz.camera=camera;
 let t0=performance.now();
 function tick(now){
  if(!viz.alive)return;
  const t=(now-t0)/1000;
  const m=viz.metrics||{};
  const pace=0.7+Math.min(1.6,(m.total?m.step/m.total:0.15));
  const queued=m.status==='queued'?0.35:1;
  fwd.forEach((mesh,i)=>place(mesh,((t*0.18*pace*queued)+i/fwd.length)%1,1));
  back.forEach((mesh,i)=>place(mesh,1-((t*0.12*pace*queued)+i/back.length)%1,-1));
  if(viz.moe&&viz.experts)viz.experts.forEach((ex,i)=>{ex.rotation.y=t*0.4+i;ex.position.y+=Math.sin(t*1.4+i)*0.002});
  if(lossMesh)lossMesh.rotation.y=t*0.7;
  renderer.render(scene,camera);
  if(!viz.reduced)viz.raf=requestAnimationFrame(tick);
 }
 renderer.render(scene,camera);
 if(!viz.reduced)viz.raf=requestAnimationFrame(tick);
}
