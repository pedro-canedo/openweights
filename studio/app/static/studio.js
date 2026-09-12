'use strict';
titles.studio = 'Data Studio';
const studio = {sources:[], jobs:[], selected:new Set(), excluded:new Set(), active:null, offset:0, total:0, query:'', split:'', role:'', parent:null};
const baseRender = render;
render = function(){
 if(state.page!=='studio'){
  baseRender();
  if(state.page==='datasets')for(const d of state.datasets.filter(d=>d.immutable)){
   const b=document.querySelector(`[data-action="append"][data-id="${d.id}"]`);
   if(b)b.outerHTML='<a class="btn small" href="#studio">Nova versão no Data Studio</a>';
  }
  if(state.page==='train')$('#train-form details .form-grid').insertAdjacentHTML('beforeend',`<div class="field"><label>Blocos de texto<select name="text_packing"><option value="pack">Concatenar documentos (eficiente)</option><option value="document">Manter documentos separados</option></select></label><small class="hint">Separar reduz mistura de documentos; pode aumentar padding. Conversas já são separadas.</small></div><div class="field"><label>Fim do documento<select name="eos_policy"><option value="append">Adicionar EOS (recomendado)</option><option value="none">Não adicionar EOS ao texto</option></select></label><small class="hint">Aplicado somente a texto livre. Padding automático usa o tokenizador da base. Datasets versionados preservam os splits publicados.</small></div>`);
  return;
 }
 disposeTrainViz();
 $('#main').classList.remove('is-playground');
 $('#main').innerHTML=studioPage();
 loadStudio().catch(e=>toast(e.message,true));
};
function studioPage(){return heading('Transforme suas fontes em aprendizado.','Adicione conteúdo, prepare os dados e revise antes de treinar.')+`
 <ol class="ds-steps" aria-label="Etapas">
  <li class="is-on"><b>1</b> Biblioteca</li>
  <li class="is-on"><b>2</b> Preparação</li>
  <li><b>3</b> Revisão</li>
  <li><b>4</b> Treinamento</li>
 </ol>
 <div class="grid-two">
  <section class="panel">
   <h2>Adicionar informações</h2>
   <p class="muted">Arquivos ficam no seu computador. Até 32 MiB por arquivo e 256 MiB por envio.</p>
   <form id="ds-upload">
    <div class="ds-drops">
     <label class="ds-drop">${ic('upload',22)}<b>Arquivos ou ZIP</b><small>PDF, texto, JSON, código</small><input type="file" name="files" multiple></label>
     <label class="ds-drop">${ic('folder',22)}<b>Pasta inteira</b><small>O navegador envia os arquivos</small><input type="file" name="folder" webkitdirectory multiple></label>
    </div>
    <details><summary>Colar texto ou JSON</summary><label>Nome e extensão<input name="name" value="literal.txt" maxlength="512"></label><label>Conteúdo<textarea name="content" rows="5" placeholder="Cole documentos ou conversas estruturadas"></textarea></label></details>
    <label>Licença ou procedência (opcional)<input name="license" maxlength="200" placeholder="Ex.: material autoral"></label>
    <button class="btn primary" type="submit">${ic('plus',14)} Adicionar à biblioteca</button>
   </form>
   <details><summary>Adicionar site ou repositório público</summary>
    <form id="ds-remote"><label>URLs HTTPS, uma por linha<textarea name="urls" rows="3" required></textarea></label><label class="check"><input type="checkbox" name="repository">Repositórios GitHub ou GitLab</label><label>Branch, tag ou commit<input name="revision" value="main"></label><p class="hint">Até 20 URLs e 32 MiB por download. Somente a página indicada é importada, sem rastrear links. Confira os direitos de uso.</p><button class="btn" type="submit">Importar URLs</button></form>
   </details>
  </section>
  <section class="panel">
   <div class="ds-lib-head"><div><h2>Biblioteca de fontes</h2><p id="ds-source-count" class="hint"></p></div><div class="flex wrap"><button class="btn small" data-ds="select-all">Selecionar todas</button><button class="btn small" data-ds="select-none">Limpar</button></div></div>
   <label>Buscar arquivos<input id="ds-source-search" type="search" placeholder="Nome, extensão ou origem"></label>
   <div id="ds-sources" class="ds-source-list"></div>
  </section>
 </div>
 <section class="panel spacer">
  <h2>Preparar dataset</h2>
  <p class="muted">Documentos viram frases empacotadas até um limite de palavras. Conversas exigem perguntas e respostas reais.</p>
  <p class="notice">A preparação é local. Classificar com um modelo frontier é opcional, exige <b>LLM_API_KEY</b> no <code>.env</code> e envia trechos ao provedor — não inventa respostas.</p>
  <form id="ds-recipe">
   <div class="form-grid">
    <label>Nome do dataset<input name="name" required value="Meu dataset" maxlength="100"></label>
    <label>Objetivo<select name="preset"><option value="documents">Documentos</option><option value="conversations">Conversas</option><option value="code">Código</option><option value="text">Texto livre</option><option value="mixed">Documentos e código mistos</option><option value="quick">Preparação rápida</option><option value="quality">Revisão de maior qualidade</option></select></label>
   </div>
   <details><summary>Configurações avançadas · qualidade, divisão e recursos</summary><div class="form-grid" id="ds-advanced"></div><p class="hint">Tokenizador, contexto, EOS e padding são aplicados no treinamento usando o modelo escolhido. A estimativa desta tela usa caracteres ÷ 4.</p></details>
   <p id="ds-parent" class="hint"></p>
   <button class="btn primary" type="submit">${ic('flask',14)} Preparar dataset</button>
  </form>
 </section>
 <section class="panel spacer">
  <h2>Revisar e publicar</h2>
  <div id="ds-jobs"></div>
  <div id="ds-review"></div>
 </section>`;}

const dsFields = [
 ['__div','Divisão do texto','','section',''],
 ['chunking','Dividir por','sentence','sentence:Frases até N palavras|paragraph:Parágrafos|fixed:Tamanho fixo|document:Documento inteiro','Frases empacotadas são o padrão para texto livre.'],
 ['max_words','Máximo de palavras por exemplo',180,'number','180 como início; uma sentença maior é partida. Código usa parágrafos.'],
 ['min_words','Mínimo de palavras',12,'number','12 descarta fragmentos sem valor de treino.'],
 ['overlap_words','Sobreposição em palavras',0,'number','0 evita repetir texto; valores maiores preservam contexto local.'],
 ['chunk_chars','Caracteres por bloco',4000,'number','Usado em parágrafos ou tamanho fixo. Ignorado no modo frases.'],
 ['overlap','Sobreposição em caracteres',0,'number','0 evita repetir texto nos modos de caracteres.'],
 ['separator','Separador de parágrafos','\\n\\n','text','Use \\n para quebra de linha no modo parágrafos.'],
 ['min_example_chars','Mínimo por exemplo',1,'number','1 preserva os trechos finais; aumente para descartar fragmentos.'],
 ['min_chars','Mínimo de caracteres',40,'number','40 recomendado; filtra o exemplo já fatiado, não o documento inteiro.'],
 ['max_chars','Máximo por exemplo',1000000,'number','1 milhão por exemplo após a divisão. Livros longos são fatiados antes deste limite.'],
 ['__qual','Qualidade e limpeza','','section',''],
 ['quality_filter','Filtro de qualidade','light','off:Desligado|light:Leve|strict:Rigoroso','Leve reconstrói o texto e remove ruído óbvio. Rigoroso aplica heurísticas extras.'],
 ['drop_boilerplate','Remover capa e metadados',false,'check','Remove catálogo, ISBN e texto-fonte. Dedicatórias literárias permanecem.'],
 ['min_quality','Nota mínima',0,'number','0 mantém tudo. Use 0,4 a 0,6 após classificação local ou externa.'],
 ['exact_dedupe','Remover duplicatas exatas',true,'check','Recomendado. Mesmo conteúdo aparece uma vez.'],
 ['near_dedupe','Remover textos quase iguais',false,'check','Mais lento; similaridade lexical, até 5 mil documentos.'],
 ['similarity','Similaridade mínima',0.9,'number','0,90 recomendado; valores menores removem mais documentos.'],
 ['normalize_spaces','Normalizar espaços',true,'check','Preserva indentação de código.'],
 ['normalize_markdown','Remover espaços finais de Markdown',false,'check','Desativado preserva quebras explícitas de Markdown.'],
 ['clean_html','Extrair texto do HTML',true,'check','Remove tags, scripts e estilos sem executar conteúdo.'],
 ['boilerplate','Remover navegação de HTML',true,'check','Remove menu, cabeçalho e rodapé estruturados.'],
 ['ocr','OCR de páginas PDF sem texto',false,'check','Opcional; usa CPU e pode produzir erros de reconhecimento.'],
 ['ocr_language','Idiomas do OCR','por+eng','por+eng:Português e inglês|por:Português|eng:Inglês|spa:Espanhol','Português e inglês como ponto de partida.'],
 ['max_pdf_pages','Máximo de páginas por PDF',10000,'number','10.000 cabe obras completas. Acima disso só as primeiras páginas entram.'],
 ['detect_language','Estimar idioma',true,'check','Heurística para PT/EN/ES; textos curtos podem ser desconhecidos.'],
 ['language','Filtro de idioma','any','any:Todos|pt:Português|en:Inglês|es:Espanhol','Todos evita descartar código e textos não reconhecidos.'],
 ['__llm','Classificação externa','','section',''],
 ['llm_classify','Classificar com modelo externo',false,'check','Opcional. Exige LLM_API_KEY no .env. Não inventa perguntas nem respostas.'],
 ['llm_model','Modelo classificador','openai/gpt-4o-mini','text','Identificador no provedor, por exemplo openai/gpt-4o-mini.'],
 ['llm_base_url','URL HTTPS do classificador','https://openrouter.ai/api/v1','text','OpenRouter ou outra API compatível. O texto sai desta máquina.'],
 ['__split','Conjuntos e grupos','','section',''],
 ['validation','Fração de validação',0.1,'number','0,10 recomendado; mede aprendizado durante o treino.'],
 ['test','Fração de teste',0.1,'number','0,10 recomendado; reservado e nunca enviado ao treino.'],
 ['grouping','Manter juntos','document','document:Mesmo documento|source:Mesma fonte / ZIP|sequence:Ordem do texto (um livro)','Documento deixa a obra inteira em um conjunto. Um único livro é dividido na ordem de leitura. Fonte mantém o ZIP junto.'],
 ['seed','Seed da divisão',42,'number','42 reproduz a mesma divisão.'],
 ['metadata','Incluir metadados por exemplo',true,'check','Recomendado para rastrear arquivo e origem.'],
 ['__tok','Tokens e conversas','','section',''],
 ['tokenizer_model_id','Modelo para contar tokens','','text','Opcional: selecione uma base já baixada. Conta tokens localmente antes do treino.'],
 ['max_tokens','Limite de tokens por exemplo',1024,'number','1.024 recomendado quando um tokenizer foi selecionado.'],
 ['long_examples','Exemplos acima do limite','reject','reject:Interromper e revisar|skip:Ignorar|truncate_text:Truncar somente texto','Interromper protege a integridade das respostas em conversas.'],
 ['instruction_field','Campo de instrução','instruction','text','Coluna da pergunta em JSON ou CSV.'],
 ['input_field','Campo de contexto','input','text','Coluna opcional de contexto.'],
 ['output_field','Campo de resposta','output','text','Coluna da resposta revisada.'],
 ['__src','Fontes e limites','','section',''],
 ['include_docs','Incluir documentação',true,'check','Recomendado para explicar o código.'],
 ['include_tests','Incluir testes',true,'check','Testes fornecem exemplos de uso.'],
 ['include_config','Incluir configurações',true,'check','Revise possíveis segredos antes de publicar.'],
 ['include_comments','Incluir comentários',true,'check','Desmarcar remove comentários de Python; demais linguagens são reportadas.'],
 ['tables','Incluir tabelas DOCX',true,'check','Preserva células como texto separado por barras.'],
 ['extensions','Extensões permitidas','','list','Vazio aceita formatos suportados. Ex.: .py, .md'],
 ['exclude','Padrões de exclusão','','list','Ex.: */fixtures/*, *.min.js; dependências e .env já são ignorados.'],
 ['invalid','Arquivo inválido','skip','skip:Ignorar e reportar|fail:Interromper','Ignorar mantém os arquivos válidos e mostra as falhas.'],
 ['max_files','Máximo de arquivos',2000,'number','2.000 recomendado; inclui arquivos ignorados em ZIP.'],
 ['max_depth','Profundidade de pastas',12,'number','12 recomendado; limita caminhos muito profundos.'],
 ['expanded_mb','Tamanho expandido (MiB)',256,'number','256 recomendado; limite total após abrir ZIPs.'],
 ['workers','Extratores paralelos',2,'number','2 recomendado na bancada; até 4, com mais consumo de RAM.'],
 ['memory_mb','Limite de memória (MiB)',4096,'number','4.096 recomendado; memória virtual do processo de preparação.'],
 ['timeout_seconds','Limite de CPU/tempo (s)',900,'number','900 recomendado; extrações individuais também têm limites.']
];
function advancedFields(){return dsFields.map(([key,label,value,type,hint])=>type==='section'?`<h3 class="ds-adv-title">${label}</h3>`:`<div class="field"><label>${type==='check'?`<input name="${key}" type="checkbox" ${value?'checked':''}> ${label}`:`${label}${type.includes('|')?`<select name="${key}">${type.split('|').map(pair=>{const [v,l]=pair.split(':');return `<option value="${v}" ${v===value?'selected':''}>${l}</option>`}).join('')}</select>`:`<input name="${key}" type="${type==='list'?'text':type}" value="${esc(value)}" ${type==='number'?'step="any"':''}>`}`}</label><small class="hint">${hint}</small></div>`).join('');}
function fileKind(name){
 const ext=(String(name||'').split('.').pop()||'').toLowerCase();
 if(['pdf'].includes(ext))return 'PDF';
 if(['txt','md','markdown','rst'].includes(ext))return 'TXT';
 if(['json','jsonl','csv'].includes(ext))return 'JSON';
 if(['docx'].includes(ext))return 'DOC';
 if(['zip'].includes(ext))return 'ZIP';
 if(['html','htm','xml'].includes(ext))return 'WEB';
 if(['py','js','ts','go','rs','java','c','cpp'].includes(ext))return 'CODE';
 return (ext||'ARQ').slice(0,4).toUpperCase();
}
const roleLabel={body:'Prosa',front_matter:'Abertura',metadata:'Metadados',boilerplate:'Ruído'};
const splitLabel={train:'Treino',validation:'Validação',test:'Teste'};
async function loadStudio(){
 const [sources,jobs]=await Promise.all([api('/studio/sources'),api('/studio/preparations')]);
 studio.sources=sources;studio.jobs=jobs;
 if(state.page!=='studio')return;
 if(!$('#ds-advanced').children.length){
  $('#ds-advanced').innerHTML=advancedFields();
  const input=$('#ds-recipe [name="tokenizer_model_id"]');
  input.outerHTML=`<select name="tokenizer_model_id"><option value="">Somente estimativa (rápido)</option>${state.models.filter(m=>m.status==='ready').map(m=>`<option value="${esc(m.id)}">${esc(m.name)}</option>`).join('')}</select>`;
 }
 paintSources();paintPreparations();
 if(studio.active)await reviewPreparation();
}
function paintSources(){
 const term=($('#ds-source-search')?.value||'').toLowerCase();
 $('#ds-source-count').textContent=`${studio.sources.length} fontes · ${studio.selected.size} selecionadas · ${fixed(studio.sources.filter(s=>studio.selected.has(s.id)).reduce((a,s)=>a+s.size,0)/2**20,1)} MiB selecionados`;
 const rows=studio.sources.filter(s=>`${s.name} ${s.origin}`.toLowerCase().includes(term));
 $('#ds-sources').innerHTML=rows.map(s=>`<label class="ds-source ${studio.selected.has(s.id)?'is-on':''}"><input type="checkbox" data-source="${s.id}" ${studio.selected.has(s.id)?'checked':''}><span class="ds-kind">${esc(fileKind(s.name))}</span><span><b>${esc(s.name)}</b><small>${fixed(s.size/1024,1)} KiB${s.license?` · ${esc(s.license)}`:''}<br>${esc(s.origin||'upload local')}</small></span></label>`).join('')||'<div class="empty"><p>Adicione arquivos para começar a biblioteca.</p></div>';
}
function paintPreparations(){
 const jobs=studio.jobs.filter(j=>j.kind==='prepare'||j.kind==='acquire');
 $('#ds-jobs').innerHTML=jobs.map(j=>{
  const actions=[
   j.kind==='prepare'&&j.status==='completed'?`<button class="btn small" data-ds="review" data-id="${j.id}">Revisar</button><button class="btn small" data-ds="version" data-id="${j.id}">Nova versão</button>`:'',
   ['queued','running'].includes(j.status)?`<button class="btn small" data-ds="cancel" data-id="${j.id}">Cancelar</button>`:'',
   j.kind==='prepare'&&['failed','cancelled','interrupted'].includes(j.status)?`<button class="btn small" data-ds="resume" data-id="${j.id}">Retomar</button><button class="btn small" data-ds="review" data-id="${j.id}">Relatório</button>`:''
  ].join('');
  const frac=j.progress&&j.progress.total?Math.min(100,Math.round(100*j.progress.done/j.progress.total)):0;
  return `<div class="ds-job"><div><div class="run-kind">${j.kind==='acquire'?'Importação':'Preparação'}</div><strong>${esc(j.name)}</strong> ${badge(j.status)}<small>${esc(j.progress?.stage||'')}${j.progress?` · ${j.progress.done}/${j.progress.total}`:''}</small>${['queued','running'].includes(j.status)?`<div class="run-track"><span style="width:${frac}%"></span></div>`:''}${j.error?`<p class="error-inline">${esc(j.error)}</p>`:''}</div><span class="ds-job-actions">${actions}</span></div>`;
 }).join('')||'<p class="muted">Suas preparações aparecerão aqui.</p>';
}
async function reviewPreparation(){
 const ident=studio.active;
 const data=await api(`/studio/preparations/${ident}?offset=${studio.offset}&limit=10&q=${encodeURIComponent(studio.query)}&split=${studio.split}&role=${encodeURIComponent(studio.role)}`);
 if(state.page!=='studio'||studio.active!==ident)return;
 studio.reviewStatus=data.job.status;
 const m=data.manifest;if(!m){$('#ds-review').innerHTML=`<p>${esc(data.job.error||data.job.progress?.stage||'Aguardando preparação…')}</p>${(data.files||[]).filter(f=>f.status==='error'||f.warning).map(f=>`<p class="notice warn">${esc(f.name)}: ${esc(f.reason||f.warning)}</p>`).join('')}`;return;}
 studio.total=data.total;
 const published=state.datasets.find(d=>d.preparation_id===ident);
 document.querySelectorAll('.ds-steps li')[2]?.classList.add('is-on');
 $('#ds-review').innerHTML=`<div class="ds-review-head"><h3>${esc(data.job.name)}</h3>${badge(data.job.status)}</div>
 <div class="metrics">
  <div class="metric"><small>Documentos</small><b>${num(m.documents)}</b></div>
  <div class="metric"><small>Exemplos</small><b>${num(m.examples)}</b></div>
  <div class="metric"><small>Tokens estimados</small><b>${num(m.estimated_tokens)}</b></div>
 </div>
 <div class="ds-split">
  <span class="ds-chip">Treino <b>${num(m.counts.train||0)}</b></span>
  <span class="ds-chip">Validação <b>${num(m.counts.validation||0)}</b></span>
  <span class="ds-chip">Teste <b>${num(m.counts.test||0)}</b></span>
  <span class="ds-chip">Filtrados <b>${num(m.filtered)}</b></span>
  <span class="ds-chip">Duplicados <b>${num(m.duplicates)}</b></span>
 </div>
 ${m.warnings.map(w=>`<p class="notice warn">${esc(w)}</p>`).join('')}
 <form id="ds-search"><label>Buscar exemplos<input name="q" value="${esc(studio.query)}"></label><label>Conjunto<select name="split">${[['','Todos'],['train','Treino'],['validation','Validação'],['test','Teste']].map(([v,l])=>`<option value="${v}" ${studio.split===v?'selected':''}>${l}</option>`).join('')}</select></label><label>Papel<select name="role">${[['','Todos'],['body','Prosa'],['front_matter','Abertura'],['metadata','Metadados'],['boilerplate','Ruído']].map(([v,l])=>`<option value="${v}" ${studio.role===v?'selected':''}>${l}</option>`).join('')}</select></label><button class="btn" type="submit">Filtrar</button></form>
 ${m.exact_tokens!=null?`<p class="notice">Contagem com o tokenizer selecionado: ${num(m.exact_tokens)} tokens. A contagem do treino pode incluir EOS e delimitadores adicionais.</p>`:''}
 ${data.rows.map(r=>{
  const role=r.metadata?.role||'';
  const q=Number(r.metadata?.quality);
  return `<article class="ds-example"><div class="ds-example-head"><label><input type="checkbox" data-exclude="${r.id}" ${studio.excluded.has(r.id)?'checked':''} ${published?'disabled':''}> Excluir</label><span class="tag split-${esc(r.split)}">${esc(splitLabel[r.split]||r.split)}</span>${role?`<span class="tag role-${esc(role)}">${esc(roleLabel[role]||role)}</span>`:''}${r.metadata?.words!=null?`<small>${num(r.metadata.words)} palavras</small>`:''}${Number.isFinite(q)?`<span class="ds-note" title="Nota ${esc(String(q))}"><i style="width:${Math.max(0,Math.min(100,q*100))}%"></i></span>`:''}<small>${esc(r.metadata?.path||r.group)}${r.metadata?.topic?` · ${esc(r.metadata.topic)}`:''}</small></div><pre>${esc(r.text||JSON.stringify(r.messages,null,2))}</pre></article>`;
 }).join('')}
 <div class="ds-pager"><button class="btn small" data-ds="previous" ${studio.offset===0?'disabled':''}>Anterior</button><span>${Math.min(studio.offset+1,data.total)}–${Math.min(studio.offset+10,data.total)} de ${data.total}</span><button class="btn small" data-ds="next" ${studio.offset+10>=data.total?'disabled':''}>Próxima</button><button class="btn small" data-ds="undo">Desfazer exclusões</button><span id="ds-excluded">${studio.excluded.size} excluídos</span></div>
 <details><summary>Relatório dos arquivos (${m.files.length})</summary><div class="ds-report table-wrap"><table><thead><tr><th>Arquivo</th><th>Status</th><th>Detalhe</th></tr></thead><tbody>${m.files.map(f=>`<tr><td>${esc(f.name)}</td><td>${esc(f.status)}</td><td>${esc(f.reason||f.warning||f.encoding||'')}</td></tr>`).join('')}</tbody></table></div></details>
 ${published?`<p class="notice">Versão ${published.version} publicada. Os conjuntos desta versão serão preservados no treinamento.</p><div class="flex wrap spacer"><a class="btn" href="/api/studio/datasets/${published.id}/dataset.jsonl">Baixar JSONL</a> <a class="btn" href="/api/studio/datasets/${published.id}/manifest.json">Manifesto</a> <button class="btn primary" data-ds="train" data-id="${published.id}">Treinar modelo</button></div>`:'<button class="btn primary spacer" data-ds="publish">Publicar dataset revisado</button>'}`;
}
document.addEventListener('input',e=>{if(e.target.id==='ds-source-search')paintSources();});
document.addEventListener('change',e=>{
 if(e.target.dataset.source){const id=e.target.dataset.source;e.target.checked?studio.selected.add(id):studio.selected.delete(id);paintSources();}
 if(e.target.dataset.exclude){const id=e.target.dataset.exclude;e.target.checked?studio.excluded.add(id):studio.excluded.delete(id);$('#ds-excluded').textContent=`${studio.excluded.size} excluídos`;}
 if(e.target.name==='preset'&&e.target.closest('#ds-recipe')) applyPreset(e.target.form,e.target.value);
});
function applyPreset(f,preset){
 const map={
  documents:{chunking:'sentence',max_words:180,quality_filter:'light',near_dedupe:false,drop_boilerplate:false,normalize_spaces:true,chunk_chars:4000},
  text:{chunking:'sentence',max_words:180,quality_filter:'light',near_dedupe:false,drop_boilerplate:false,normalize_spaces:true,chunk_chars:4000},
  mixed:{chunking:'sentence',max_words:180,quality_filter:'light',near_dedupe:false,drop_boilerplate:false,normalize_spaces:true,chunk_chars:4000},
  quality:{chunking:'sentence',max_words:256,quality_filter:'strict',near_dedupe:true,drop_boilerplate:true,normalize_spaces:true,chunk_chars:4000},
  quick:{chunking:'sentence',max_words:128,quality_filter:'off',near_dedupe:false,drop_boilerplate:false,normalize_spaces:true,chunk_chars:2000},
  code:{chunking:'paragraph',quality_filter:'off',near_dedupe:false,drop_boilerplate:false,normalize_spaces:false,chunk_chars:4000},
  conversations:{chunking:'document',quality_filter:'off',near_dedupe:false,drop_boilerplate:false,normalize_spaces:true,chunk_chars:4000}
 };
 const values=map[preset];if(!values)return;
 Object.entries(values).forEach(([key,value])=>{
  const el=f.elements[key];if(!el)return;
  if(el.type==='checkbox')el.checked=value;else el.value=value;
 });
}
document.addEventListener('submit',async e=>{
 const f=e.target;if(!f.id.startsWith('ds-'))return;e.preventDefault();const b=f.querySelector('[type="submit"]');b.disabled=true;
 try{
  if(f.id==='ds-upload'){
   const fd=new FormData();fd.set('content',f.elements.content.value);fd.set('name',f.elements.name.value);fd.set('license',f.elements.license.value);
   for(const file of [...f.elements.files.files,...f.elements.folder.files])fd.append('files',file,file.webkitRelativePath||file.name);
   const added=await api('/studio/sources',{method:'POST',body:fd});added.forEach(s=>studio.selected.add(s.id));f.reset();await loadStudio();toast('Fontes preservadas na biblioteca. Agora prepare seu dataset.');
  }else if(f.id==='ds-remote'){
   await post('/studio/remote',{urls:f.elements.urls.value.split('\n').map(s=>s.trim()).filter(Boolean),repository:f.elements.repository.checked,revision:f.elements.revision.value});await loadStudio();toast('Importação adicionada à fila.');
  }else if(f.id==='ds-recipe'){
   const recipe={name:f.elements.name.value,preset:f.elements.preset.value,source_ids:[...studio.selected],parent_id:studio.parent};
   for(const [key,, ,type] of dsFields){
    if(type==='section')continue;
    const el=f.elements[key];if(!el)continue;
    recipe[key]=type==='check'?el.checked:type==='number'?Number(el.value):type==='list'?el.value.split(',').map(s=>s.trim()).filter(Boolean):el.value;
   }
   recipe.separator=recipe.separator.replace(/\\n/g,'\n');
   recipe.tokenizer_model_id=recipe.tokenizer_model_id||null;
   recipe.llm_classify=!!recipe.llm_classify;
   recipe.drop_boilerplate=!!recipe.drop_boilerplate;
   const job=await post('/studio/prepare',recipe);studio.active=job.id;studio.offset=0;studio.query='';studio.split='';studio.role='';studio.excluded.clear();await loadStudio();toast('Preparação na fila. Você pode continuar usando a aplicação.');
  }else if(f.id==='ds-search'){studio.query=f.elements.q.value;studio.split=f.elements.split.value;studio.role=f.elements.role.value;studio.offset=0;await reviewPreparation();}
 }catch(error){toast(error.message,true);}finally{b.disabled=false;}
});
document.addEventListener('click',async e=>{
 const b=e.target.closest('[data-ds]');if(!b)return;b.disabled=true;
 try{
  const action=b.dataset.ds,id=b.dataset.id;
  if(action==='select-all'){studio.sources.forEach(s=>studio.selected.add(s.id));paintSources();}
  if(action==='select-none'){studio.selected.clear();paintSources();}
 if(action==='review'){studio.active=id;studio.offset=0;studio.excluded.clear();await reviewPreparation();$('#ds-review').scrollIntoView({behavior:'smooth'});}
  if(action==='undo'){studio.excluded.clear();await reviewPreparation();}
  if(action==='next'||action==='previous'){studio.offset=Math.max(0,studio.offset+(action==='next'?10:-10));await reviewPreparation();}
  if(action==='cancel'){await post('/jobs/'+id+'/cancel');await loadStudio();}
  if(action==='resume'){await post('/studio/preparations/'+id+'/resume');await loadStudio();}
  if(action==='publish'){await post('/studio/preparations/'+studio.active+'/publish',{excluded_ids:[...studio.excluded]});await refresh();await reviewPreparation();toast('Dataset publicado. Pronto para configurar o treinamento.');}
  if(action==='version'){
   const job=studio.jobs.find(j=>j.id===id),f=$('#ds-recipe');studio.parent=state.datasets.find(d=>d.preparation_id===id)?.id||null;studio.selected=new Set(job.config.source_ids);
   for(const [key,value] of Object.entries(job.config)){const el=f.elements[key];if(!el)continue;if(el.type==='checkbox')el.checked=value;else el.value=Array.isArray(value)?value.join(', '):key==='separator'?value.replace(/\n/g,'\\n'):value;}
   $('#ds-parent').textContent=studio.parent?'Nova versão: a anterior será preservada.':'Nova receita a partir desta preparação.';paintSources();f.scrollIntoView({behavior:'smooth'});
  }
  if(action==='train'){
   const dataset=state.datasets.find(d=>d.id===id);location.hash='train';setTimeout(()=>{const f=$('#train-form');if(!f)return;f.elements.mode.value=dataset.kind==='chat'?'qlora':'continued';changeMode();f.elements.dataset_id.value=id;toast('Dataset selecionado. Escolha a base ou o modo de estudo do zero.');},0);
  }
 }catch(error){toast(error.message,true);}finally{b.disabled=false;}
});
setInterval(async()=>{if(state.page!=='studio')return;try{studio.jobs=await api('/studio/preparations');if(state.page==='studio'){paintPreparations();const j=studio.jobs.find(j=>j.id===studio.active);if(j&&j.status!==studio.reviewStatus)await reviewPreparation();const sources=await api('/studio/sources');if(state.page==='studio'&&sources.length!==studio.sources.length){studio.sources=sources;paintSources();}}}catch(error){toast(error.message,true);}},3000);
if(location.hash.slice(1)==='studio')route();
