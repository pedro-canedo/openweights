"""A single, disposable GPU worker for download, training or generation."""
import hashlib
import importlib.metadata
import json
import math
import os
import shutil
import signal
import subprocess
import sys
import time
import traceback
from pathlib import Path
from . import store, runtime

CANCELLED = False


def interrupt(signum, frame):
    global CANCELLED
    CANCELLED = True
    print('Cancelamento solicitado. Salvando ao final do próximo passo, quando possível.', flush=True)


def check_cancel():
    global CANCELLED
    if os.environ.get('OW_STUDIO_JOB'):
        CANCELLED = CANCELLED or store.get('jobs', os.environ['OW_STUDIO_JOB']).get('cancel_requested', False)
    if CANCELLED:
        raise InterruptedError('Trabalho cancelado.')


def resolve_model(record):
    if record['source'] == 'local':
        path = (runtime.models_dir() / record['repo']).resolve()
        if not path.is_relative_to(runtime.models_dir()):
            raise ValueError('Caminho local inválido.')
        return str(path)
    from huggingface_hub import snapshot_download
    from tqdm.auto import tqdm
    class DownloadProgress(tqdm):
        def update(self, n=1):
            result = super().update(n)
            ident = os.environ.get('OW_STUDIO_JOB')
            if ident and self.total:
                store.update('jobs', ident, {'download_progress': {'received_files': self.n, 'total_files': self.total}})
            return result
    print(f"Carregando {record['repo']} ({record.get('resolved_revision') or record['revision']})", flush=True)
    path = snapshot_download(record['repo'], revision=record.get('resolved_revision') or record['revision'],
        allow_patterns=['*.json','*.safetensors','*.model','*.txt','*.tiktoken','*.jinja','*.jinja2'],
        max_workers=4, tqdm_class=DownloadProgress)
    if not list(Path(path).glob('*.safetensors')):
        raise ValueError('Repositório não contém pesos safetensors compatíveis. GGUF não é um checkpoint de treino.')
    store.update('models', record['id'], {'status':'ready','resolved_revision':Path(path).name})
    check_cancel()
    return path


def gpu_setup(seed):
    import torch
    from transformers import set_seed
    if not torch.cuda.is_available():
        raise RuntimeError('Não foi possível usar a GPU. Atualize o driver NVIDIA e repare o módulo de treinamento.')
    set_seed(seed)
    torch.set_num_threads(4)
    torch.backends.cuda.matmul.allow_tf32 = True
    torch.backends.cudnn.allow_tf32 = True
    torch.cuda.reset_peak_memory_stats()
    bf16 = torch.cuda.is_bf16_supported()
    print('GPU:', torch.cuda.get_device_name(0), '| precisão:', 'BF16' if bf16 else 'FP16', flush=True)
    return torch.bfloat16 if bf16 else torch.float16, bf16


def train(job, folder):
    import torch
    from datasets import Dataset
    from transformers import (AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig,
        Trainer, TrainerCallback, TrainingArguments, DataCollatorForLanguageModeling,
        GPT2Config, GPT2LMHeadModel, PreTrainedTokenizerFast)
    c = job['config']
    dtype, bf16 = gpu_setup(c['seed'])
    rows = json.loads((folder/'dataset.json').read_text(encoding='utf-8'))
    saved_config = json.loads((folder/'config.json').read_text(encoding='utf-8'))
    model_record = saved_config.get('model')
    timer = time.monotonic()
    checkpoint_path = folder/job['resume'] if job.get('resume') else None
    model_path = None
    if c['mode'] in ('scratch', 'scratch_moe'):
        from tokenizers import Tokenizer, models, trainers, pre_tokenizers, decoders
        tokpath = folder/'tokenizer'
        if checkpoint_path and tokpath.exists():
            tok = AutoTokenizer.from_pretrained(tokpath)
        else:
            raw = Tokenizer(models.BPE(unk_token='[UNK]'))
            raw.pre_tokenizer = pre_tokenizers.ByteLevel(add_prefix_space=False)
            raw.decoder = decoders.ByteLevel()
            raw.train_from_iterator((r['text'] for r in rows['train']), trainers.BpeTrainer(
                vocab_size=4096 if c['scratch_size']=='tiny' else 16000,
                min_frequency=2, special_tokens=['[PAD]','[UNK]','[BOS]','[EOS]'],
                initial_alphabet=pre_tokenizers.ByteLevel.alphabet()))
            tok = PreTrainedTokenizerFast(tokenizer_object=raw, pad_token='[PAD]', unk_token='[UNK]',
                                         bos_token='[BOS]', eos_token='[EOS]')
            tok.save_pretrained(tokpath)
        tiny = c['scratch_size'] == 'tiny'
        if c['mode'] == 'scratch_moe':
            from .moe import MoEConfig, MoEForCausalLM
            config = MoEConfig(
                vocab_size=len(tok),
                hidden_size=128 if tiny else 256,
                num_hidden_layers=2 if tiny else 4,
                num_attention_heads=4,
                intermediate_size=384 if tiny else 768,
                max_position_embeddings=c['context'],
                num_experts=c['num_experts'],
                top_k=c['top_k'],
                capacity_factor=c['capacity_factor'],
                router_aux_loss_coef=c['router_aux_loss_coef'],
                bos_token_id=tok.bos_token_id,
                eos_token_id=tok.eos_token_id,
                pad_token_id=tok.pad_token_id,
            )
            model = MoEForCausalLM(config)
        else:
            config = GPT2Config(vocab_size=len(tok), n_positions=c['context'], n_ctx=c['context'],
                n_embd=128 if tiny else 512, n_layer=2 if tiny else 8, n_head=4 if tiny else 8,
                bos_token_id=tok.bos_token_id, eos_token_id=tok.eos_token_id,
                pad_token_id=tok.pad_token_id, use_cache=False)
            config._attn_implementation = 'sdpa'
            model = GPT2LMHeadModel(config)
    else:
        # Pin the first resolved revision in the immutable run configuration for resume/export.
        model_path = resolve_model(model_record)
        if model_record['source']=='hub' and not c.get('one_epoch'):
            model_record['resolved_revision'] = Path(model_path).name
            saved_config['model'] = model_record
            store.write_json(folder/'config.json', saved_config)
        tok = AutoTokenizer.from_pretrained(model_path, trust_remote_code=False)
        quant = BitsAndBytesConfig(load_in_4bit=True, bnb_4bit_quant_type='nf4',
            bnb_4bit_use_double_quant=True, bnb_4bit_compute_dtype=dtype)
        model = AutoModelForCausalLM.from_pretrained(model_path, quantization_config=quant,
            torch_dtype=dtype, device_map={'':0}, attn_implementation='sdpa',
            trust_remote_code=False, use_safetensors=True)
        from peft import prepare_model_for_kbit_training, get_peft_model, LoraConfig
        model = prepare_model_for_kbit_training(model, use_gradient_checkpointing=c['gradient_checkpointing'])
        model = get_peft_model(model, LoraConfig(r=c['rank'], lora_alpha=2*c['rank'],
            lora_dropout=0.05, target_modules='all-linear', bias='none', task_type='CAUSAL_LM'))
        model.print_trainable_parameters()
    check_cancel()
    if tok.eos_token_id is None:
        raise ValueError('Tokenizer precisa de token de fim de sequência.')
    if tok.pad_token_id is None:
        tok.pad_token = tok.eos_token
    tok.padding_side = 'right'
    model.config.use_cache = False
    cap = getattr(model.config, 'max_position_embeddings', c['context'])
    if c['context'] > cap:
        raise ValueError(f'Contexto escolhido excede o limite de {cap} do modelo.')
    token_counts = {}

    effective_steps = c['max_steps']
    class Monitor(TrainerCallback):
        def on_save(self, args, state, control, **kwargs):
            if c.get('one_epoch'):
                checkpoint = folder/f'checkpoint-{state.global_step}'
                hashes = {}
                for path in checkpoint.rglob('*'):
                    if path.is_file() and path.name != 'complete.json':
                        with path.open('rb') as stream:
                            hashes[path.relative_to(checkpoint).as_posix()] = hashlib.file_digest(stream, 'sha256').hexdigest()
                store.write_json(checkpoint/'complete.json', {'step': state.global_step, 'sha256': hashes, 'runtime': runtime.runtime_id()})
        def on_log(self, args, state, control, logs=None, **kwargs):
            entry = {k: float(v) if isinstance(v, (int,float)) else str(v) for k,v in (logs or {}).items()}
            tracked_model = kwargs.get('model')
            if tracked_model is not None and getattr(tracked_model, 'last_router_aux_loss', None) is not None:
                entry['router_aux_loss'] = tracked_model.last_router_aux_loss
                entry['expert_counts'] = tracked_model.last_expert_counts
            entry.update(step=state.global_step, total_steps=min(state.max_steps,effective_steps), elapsed=time.monotonic()-timer,
                vram_gb=torch.cuda.max_memory_allocated()/2**30)
            with (folder/'metrics.jsonl').open('a', encoding='utf-8') as f:
                f.write(json.dumps(entry, ensure_ascii=False)+'\n')
        def on_step_end(self, args, state, control, **kwargs):
            global CANCELLED
            CANCELLED = CANCELLED or store.get('jobs', job['id']).get('cancel_requested', False)
            if CANCELLED or (c.get('one_epoch') and (state.global_step >= effective_steps or time.monotonic()-timer >= c.get('max_minutes', 1440)*60)):
                control.should_training_stop = True
                control.should_save = True
            return control

    interval = min(c['save_steps'], c['max_steps'])
    common = dict(output_dir=str(folder), per_device_train_batch_size=c['batch'],
        per_device_eval_batch_size=1, gradient_accumulation_steps=c['accumulation'],
        learning_rate=c['learning_rate'], max_steps=-1 if c.get('one_epoch') else c['max_steps'], num_train_epochs=1,
        warmup_ratio=0.03, lr_scheduler_type='cosine', weight_decay=0.01, max_grad_norm=1.0,
        bf16=bf16, fp16=not bf16, tf32=True,
        gradient_checkpointing=c['gradient_checkpointing'] and c['mode'] != 'scratch_moe',
        optim='adamw_torch_fused' if c['mode'] in ('scratch', 'scratch_moe') else 'paged_adamw_8bit',
        eval_strategy='steps', eval_steps=interval, save_strategy='steps', save_steps=interval,
        save_total_limit=2, load_best_model_at_end=True, metric_for_best_model='eval_loss',
        greater_is_better=False, logging_steps=1, report_to='none', seed=c['seed'],
        dataloader_num_workers=0, save_safetensors=True, disable_tqdm=True)
    if c['mode']=='qlora':
        from trl import SFTConfig, SFTTrainer
        if not tok.chat_template:
            raise ValueError('Este modelo não possui chat template. Use um modelo Instruct compatível.')
        def conversations(records, split):
            examples = []
            tokens = 0
            for i, row in enumerate(records):
                messages = row['messages']
                ids = tok.apply_chat_template(messages, tokenize=True)
                if len(ids)>c['context']:
                    raise ValueError(f'{split}: exemplo {i+1} tem {len(ids)} tokens, acima de {c["context"]}. Aumente contexto ou encurte o exemplo.')
                tokens += len(ids)
                examples.append({'prompt':messages[:-1], 'completion':messages[-1:]})
            token_counts[split] = tokens
            return Dataset.from_list(examples)
        train_ds = conversations(rows['train'], 'train')
        val_ds = conversations(rows['validation'], 'validation')
        args = SFTConfig(**common, max_length=c['context'], packing=False,
                         completion_only_loss=True, eos_token=tok.eos_token)
        trainer = SFTTrainer(model=model, args=args, train_dataset=train_ds,
            eval_dataset=val_ds, processing_class=tok, callbacks=[Monitor()])
        batch = trainer.data_collator([trainer.train_dataset[0]])
        labels = batch['labels'][0]
        if not (labels!=-100).any():
            raise ValueError('Nenhum token supervisionado. Revise o chat template.')
        print('Alvo de exemplo:', tok.decode(labels[labels!=-100].tolist())[:400])
    else:
        def blocks(records, split):
            examples, buffer, total = [], [], 0
            for row in records:
                ids = tok.encode(row['text'], add_special_tokens=False)
                if c.get('eos_policy', 'append') == 'append':
                    ids += [tok.eos_token_id]
                total += len(ids)
                if c.get('text_packing', 'pack') == 'document':
                    for start in range(0, len(ids), c['context']):
                        chunk = ids[start:start+c['context']]
                        if len(chunk) >= 2:
                            examples.append({'input_ids':chunk, 'attention_mask':[1]*len(chunk)})
                    continue
                buffer.extend(ids)
                end = len(buffer)//c['context']*c['context']
                for start in range(0,end,c['context']):
                    chunk = buffer[start:start+c['context']]
                    examples.append({'input_ids':chunk, 'attention_mask':[1]*len(chunk)})
                buffer=buffer[end:]
            if len(buffer)>=2:
                examples.append({'input_ids':buffer,'attention_mask':[1]*len(buffer)})
            if not examples:
                raise ValueError(f'{split}: texto insuficiente para tokenizar.')
            token_counts[split]=total
            return Dataset.from_list(examples)
        train_ds, val_ds = blocks(rows['train'],'train'), blocks(rows['validation'],'validation')
        # Mask only actual padding, preserving EOS even when pad and EOS share an ID.
        def collator(features):
            batch = tok.pad(features, padding=True, return_tensors='pt')
            batch['labels'] = batch['input_ids'].clone()
            batch['labels'][batch['attention_mask']==0]=-100
            return batch
        trainer = Trainer(model=model, args=TrainingArguments(**common), train_dataset=train_ds,
            eval_dataset=val_ds, processing_class=tok, data_collator=collator, callbacks=[Monitor()])
    manifest = {'base':model_record, 'mode':c['mode'], 'token_counts':token_counts,
        'dataset_sha256':hashlib.sha256((folder/'dataset.json').read_bytes()).hexdigest(),
        'gpu':torch.cuda.get_device_name(0), 'torch':torch.__version__, 'cuda':torch.version.cuda,
        'packages':{n:importlib.metadata.version(n) for n in ['transformers','trl','peft','bitsandbytes','datasets']},
        'parameters':sum(p.numel() for p in model.parameters()),
        'trainable_parameters':sum(p.numel() for p in model.parameters() if p.requires_grad),
        'quantized':c['mode'] not in ('scratch', 'scratch_moe'),
        'precision':'bf16' if bf16 else 'fp16',
        'architecture':'moe' if c['mode'] == 'scratch_moe' else 'dense',
        'moe': ({'num_experts':c['num_experts'], 'top_k':c['top_k'],
                 'capacity_factor':c['capacity_factor'],
                 'router_aux_loss_coef':c['router_aux_loss_coef']}
                if c['mode'] == 'scratch_moe' else None)}
    store.write_json(folder/'manifest.json', manifest)
    store.update('jobs', job['id'], {'training_info':manifest})
    print('Tokens:', token_counts, '| batch efetivo:', c['batch']*c['accumulation'], flush=True)
    check_cancel()
    effective_steps = min(c['max_steps'], max(1, math.ceil(len(train_ds)/(c['batch']*c['accumulation'])))) if c.get('one_epoch') else c['max_steps']
    benchmark_file = folder/'benchmark.json'
    if c.get('recipe_id') == 'recommended' and benchmark_file.exists():
        measured = json.loads(benchmark_file.read_text(encoding='utf-8'))
        seconds = measured.get('train', {}).get('train_runtime', 0)
        if seconds > 0:
            effective_steps = min(effective_steps, max(1, int(c.get('max_minutes',30)*60 / seconds)))
    store.update('jobs', job['id'], {'progress': {'total_steps': effective_steps, 'training_started_at': store.now()}})
    result = trainer.train(resume_from_checkpoint=str(checkpoint_path) if checkpoint_path else None)
    if CANCELLED:
        print('Treino interrompido; checkpoint preservado.')
        return
    artifact = folder/'artifact'
    trainer.save_model(str(artifact))
    tok.save_pretrained(artifact)
    metrics = trainer.evaluate()
    if c.get('one_epoch') and rows.get('test'):
        store.update('jobs', job['id'], {'stage': 'evaluating'})
        test_ds = conversations(rows['test'], 'test') if c['mode'] == 'qlora' else blocks(rows['test'], 'test')
        if c['mode'] == 'qlora':
            test_ds = trainer._prepare_dataset(test_ds, tok, trainer.args, trainer.args.packing, None, 'test')
        metrics['held_out_test'] = trainer.evaluate(eval_dataset=test_ds, metric_key_prefix='test')
    metrics['peak_vram_gb'] = torch.cuda.max_memory_allocated()/2**30
    metrics['duration_seconds'] = time.monotonic()-timer
    metrics['train'] = result.metrics
    if c.get('one_epoch'):
        manifest.update(runtime=runtime.runtime_id(), recipe=c, evaluation=metrics,
                        limitation='Treino curto não garante respostas factuais sobre o documento.')
        manifest['artifact_sha256'] = {}
        for path in artifact.rglob('*'):
            if path.is_file():
                with path.open('rb') as stream:
                    manifest['artifact_sha256'][path.relative_to(artifact).as_posix()] = hashlib.file_digest(stream, 'sha256').hexdigest()
        store.write_json(folder/'manifest.json', manifest)
        store.write_json(folder/'training-complete.json', {'artifact_sha256': manifest['artifact_sha256']})
        store.update('jobs', job['id'], {'training_info': manifest})
    store.update('jobs', job['id'], {'result':metrics,'artifact':str(artifact)})
    print('Treinamento concluído. Artefato:', artifact, flush=True)


def generate(job, folder):
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig
    c = job['config']
    dtype, _ = gpu_setup(42)
    trained = store.get('jobs', c['run_id']) if c.get('run_id') else None
    artifact = store.run_dir(trained['id'])/'artifact' if trained else None
    scratch = trained and trained['config']['mode'] in ('scratch', 'scratch_moe')
    if scratch:
        config_data = json.loads((artifact / 'config.json').read_text(encoding='utf-8'))
        if config_data.get('model_type') == 'llm_studio_moe':
            from .moe import MoEForCausalLM
            model = MoEForCausalLM.from_pretrained(artifact, torch_dtype=dtype).to('cuda')
        else:
            model = AutoModelForCausalLM.from_pretrained(artifact, torch_dtype=dtype, use_safetensors=True).to('cuda')
        tok = AutoTokenizer.from_pretrained(artifact)
    else:
        record = json.loads((store.run_dir(trained['id'])/'config.json').read_text())['model'] if trained else store.get('models', c['model_id'])
        path = resolve_model(record)
        tok = AutoTokenizer.from_pretrained(artifact or path, trust_remote_code=False)
        quant = BitsAndBytesConfig(load_in_4bit=True, bnb_4bit_quant_type='nf4',
            bnb_4bit_use_double_quant=True, bnb_4bit_compute_dtype=dtype)
        model = AutoModelForCausalLM.from_pretrained(path, torch_dtype=dtype, device_map={'':0},
            quantization_config=quant, attn_implementation='sdpa', trust_remote_code=False, use_safetensors=True)
        if trained:
            from peft import PeftModel
            model = PeftModel.from_pretrained(model, artifact)
    check_cancel()
    model.eval()
    model.config.use_cache=True
    messages = ([{'role':'system','content':c['system']}] if c['system'] else [])+[{'role':'user','content':c['prompt']}]
    text = tok.apply_chat_template(messages, tokenize=False, add_generation_prompt=True) if tok.chat_template and not scratch else c['prompt']
    inputs=tok(text, return_tensors='pt', add_special_tokens=False).to('cuda')
    limit = getattr(model.config,'max_position_embeddings',8192)
    budget = min(c['max_tokens'], limit-inputs['input_ids'].shape[1])
    if budget<=0:
        raise ValueError('Prompt excede o contexto disponível do modelo.')
    start=time.monotonic()
    options={'do_sample':c['temperature']>0}
    if options['do_sample']: options['temperature']=c['temperature']
    from transformers import StoppingCriteria, StoppingCriteriaList
    class CancelGeneration(StoppingCriteria):
        def __call__(self, input_ids, scores, **kwargs):
            return CANCELLED or store.get('jobs', job['id']).get('cancel_requested', False)
    with torch.inference_mode():
        output=model.generate(**inputs, max_new_tokens=budget, **options,
            pad_token_id=tok.pad_token_id if tok.pad_token_id is not None else tok.eos_token_id,
            eos_token_id=tok.eos_token_id, stopping_criteria=StoppingCriteriaList([CancelGeneration()]))
    generated=output[0,inputs['input_ids'].shape[1]:]
    result={'text':tok.decode(generated, skip_special_tokens=True), 'tokens':len(generated),
            'seconds':time.monotonic()-start, 'peak_vram_gb':torch.cuda.max_memory_allocated()/2**30}
    store.update('jobs',job['id'],{'result':result})
    store.write_json(folder/'result.json',result)


def gguf_export(job, folder):
    """Merge an adapter (when needed), convert to GGUF and optionally quantize it.

    The exporter deliberately runs on CPU.  Training artifacts remain immutable in
    their source run directory and the generated GGUF plus manifest live in the
    export job directory, so a failed conversion can be retried safely.
    """
    source_id = job['config']['run_id']
    source = store.get('jobs', source_id)
    if source['kind'] != 'train' or source['status'] != 'completed':
        raise ValueError('A exportação GGUF exige um treinamento concluído.')
    mode = source['config'].get('mode')
    if mode == 'scratch':
        raise ValueError('A LLM do zero usa um tokenizer BPE próprio que o llama.cpp não reconhece.')
    if mode == 'scratch_moe':
        raise ValueError('O MoE customizado ainda precisa de um conversor GGUF próprio.')

    converter = runtime.converter()
    quantizer = runtime.quantizer()
    if not converter.is_file() or not quantizer.is_file():
        raise ValueError('Ferramentas GGUF indisponíveis. Repare o módulo de treinamento.')

    check_cancel()
    source_folder = store.run_dir(source.get('source_workflow', source_id))
    source_artifact = source_folder / 'artifact'
    if not source_artifact.is_dir():
        raise ValueError('O treinamento concluído não possui um artefato.')
    work = folder / 'gguf-work'
    model_dir = work / 'model'
    work.mkdir(parents=True, exist_ok=True)
    model_dir.mkdir(parents=True, exist_ok=True)

    # A scratch LLM already is a complete Transformers checkpoint.  QLoRA and
    # continued runs only contain an adapter, therefore merge it with the exact
    # base revision frozen in that run's config before conversion.
    if mode in ('qlora', 'continued'):
        import gc
        import torch
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer

        saved = json.loads((source_folder / 'config.json').read_text(encoding='utf-8'))
        record = saved.get('model')
        if not record:
            raise ValueError('A configuração do treinamento não registra a base do modelo.')
        base_path = resolve_model(record)
        import psutil
        import shutil
        weight_bytes = sum(path.stat().st_size for path in Path(base_path).rglob('*.safetensors'))
        required_ram = max(4 * 2**30, weight_bytes * 3)
        required_disk = max(4 * 2**30, weight_bytes * 4)
        if psutil.virtual_memory().available < required_ram:
            raise ValueError(f'Falta memória RAM para gerar o modelo. Feche outros aplicativos para liberar {required_ram / 2**30:.1f} GB e tente exportar novamente.')
        if shutil.disk_usage(work).free < required_disk:
            raise ValueError(f'Falta espaço para gerar o modelo. Libere {required_disk / 2**30:.1f} GB no disco do Studio e tente exportar novamente.')
        print('Mesclando adaptador LoRA em CPU:', base_path, flush=True)
        base = AutoModelForCausalLM.from_pretrained(
            base_path, torch_dtype=torch.float16, device_map='cpu',
            low_cpu_mem_usage=True, trust_remote_code=False, use_safetensors=True)
        adapter = PeftModel.from_pretrained(base, source_artifact)
        merged = adapter.merge_and_unload()
        merged.save_pretrained(model_dir, safe_serialization=True, max_shard_size='2GB')
        tok = AutoTokenizer.from_pretrained(source_artifact, trust_remote_code=False)
        tok.save_pretrained(model_dir)
        del merged, adapter, base
        gc.collect()
    else:
        raise ValueError(f'Modo de treinamento incompatível com GGUF: {mode}.')

    check_cancel()
    requested = str(job['config'].get('quantization', 'Q4_K_M')).upper()
    if requested not in ('F16', 'BF16', 'Q8_0', 'Q5_K_M', 'Q4_K_M'):
        raise ValueError(f'Quantização GGUF não suportada: {requested}.')
    # llama.cpp quantizes from an f16/bf16 intermediate.  Keeping that
    # intermediate for all quantized variants produces deterministic output.
    outtype = 'bf16' if requested == 'BF16' else 'f16'
    converted = work / f'{source_id}.{outtype}.gguf'
    final = folder / f'open-weights-{source_id[:8]}-{requested.lower()}.gguf'
    env = os.environ.copy()
    env['PYTHONPATH'] = str(runtime.llama_dir() / 'gguf-py')
    convert_cmd = [sys.executable, str(converter), '--outfile', str(converted),
                   '--outtype', outtype, str(model_dir)]
    print('Convertendo Transformers → GGUF:', ' '.join(convert_cmd), flush=True)
    hidden = {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}
    subprocess.run(convert_cmd, check=True, cwd=runtime.llama_dir(), env=env, **hidden)
    check_cancel()
    if requested in ('F16', 'BF16'):
        converted.replace(final)
    else:
        quant_cmd = [str(quantizer), str(converted), str(final), requested]
        print('Quantizando GGUF:', ' '.join(quant_cmd), flush=True)
        subprocess.run(quant_cmd, check=True, cwd=runtime.llama_dir(), env=env, **hidden)
        converted.unlink(missing_ok=True)
    if not final.is_file():
        raise ValueError('A ferramenta GGUF não produziu o arquivo esperado.')
    digest = hashlib.sha256()
    with final.open('rb') as stream:
        for chunk in iter(lambda: stream.read(8 * 1024 * 1024), b''):
            digest.update(chunk)
    manifest = {
        'run_id': source_id,
        'quantization': requested,
        'artifact': str(final),
        'size_bytes': final.stat().st_size,
        'sha256': digest.hexdigest(),
        'llama_cpp_ref': os.getenv('LLAMA_CPP_REF', 'b95502ba9aa0eb73a2f4fc8878d7fbe6a847a0b9'),
        'base': source.get('training_info', {}).get('base'),
        'mode': mode,
    }
    store.write_json(folder / 'gguf-manifest.json', manifest)
    store.update('jobs', job['id'], {'artifact': str(final), 'result': manifest})
    shutil.rmtree(work, ignore_errors=True)
    print('GGUF concluído:', final, '| SHA-256:', manifest['sha256'], flush=True)


def main():
    os.environ['PYTHONIOENCODING'] = 'utf-8'
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, 'reconfigure'):
            stream.reconfigure(encoding='utf-8', errors='replace')
    ident=sys.argv[1]
    os.environ['OW_STUDIO_JOB'] = ident
    job=store.get('jobs',ident)
    folder=store.run_dir(ident)
    signal.signal(signal.SIGTERM,interrupt)
    signal.signal(signal.SIGINT,interrupt)
    try:
        if job['kind'] in ('prepare', 'acquire'):
            from .studio.pipeline import prepare, acquire
            if sys.platform == 'linux':
                import resource
                resource.setrlimit(resource.RLIMIT_AS, (job['config'].get('memory_mb', 4096)*1024**2,)*2)
                resource.setrlimit(resource.RLIMIT_CPU, (job['config'].get('timeout_seconds', 900),)*2)
            (prepare if job['kind'] == 'prepare' else acquire)(job, folder, check_cancel)
        elif job['kind']=='guided_prepare':
            from .guided import prepare
            prepare(job, folder, check_cancel)
        elif job['kind']=='workflow':
            from .workflow import run
            from .gpu_lease import lease
            with lease():
                run(job, folder, check_cancel)
        elif job['kind']=='train':
            from .gpu_lease import lease
            with lease(): train(job,folder)
        elif job['kind']=='generate':
            from .gpu_lease import lease
            with lease(): generate(job,folder)
        elif job['kind']=='comparison':
            from .gpu_lease import lease
            from .workflow import compare_models
            with lease(): compare_models(job, folder, check_cancel)
        elif job['kind']=='gguf': gguf_export(job,folder)
        elif job['kind']=='download':
            path=resolve_model(store.get('models',job['config']['model_id']))
            store.update('jobs',ident,{'result':{'path':path}})
    except Exception as exc:
        error=str(exc)
        if isinstance(exc, MemoryError) and job['kind'] in ('prepare', 'acquire'):
            error='Limite de RAM da preparação excedido. Reduza o lote ou aumente a memória nas configurações avançadas.'
        token=os.getenv('HF_TOKEN')
        if token: error=error.replace(token,'[TOKEN]')
        if 'out of memory' in error.lower():
            error='VRAM insuficiente. Reduza contexto/batch, feche outros processos ou escolha modelo menor. '+error
        store.update('jobs',ident,{'error':error})
        print('ERRO:',error,flush=True)
        # Traceback without exception text avoids leaking credentials from upstream errors.
        traceback.print_tb(exc.__traceback__)
        sys.exit(1)


if __name__=='__main__':
    main()
