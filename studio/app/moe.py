"""Small, educational Mixture-of-Experts causal language model.

The implementation is intentionally compact so it can be trained on a single
24 GB GPU. Each token is routed to ``top_k`` feed-forward experts while the
self-attention path stays shared. It is a real Hugging Face ``PreTrainedModel``
so checkpoints can be saved, resumed and generated with the normal APIs.
"""
from dataclasses import dataclass
from typing import Optional

import torch
from torch import nn
from transformers import GenerationMixin, PretrainedConfig, PreTrainedModel
from transformers.modeling_outputs import ModelOutput


class MoEConfig(PretrainedConfig):
    model_type = "llm_studio_moe"

    def __init__(
        self,
        vocab_size: int = 4096,
        hidden_size: int = 256,
        num_hidden_layers: int = 4,
        num_attention_heads: int = 4,
        intermediate_size: int = 768,
        max_position_embeddings: int = 512,
        num_experts: int = 4,
        top_k: int = 2,
        capacity_factor: float = 1.25,
        router_aux_loss_coef: float = 0.01,
        **kwargs,
    ):
        super().__init__(**kwargs)
        self.vocab_size = vocab_size
        self.hidden_size = hidden_size
        self.num_hidden_layers = num_hidden_layers
        self.num_attention_heads = num_attention_heads
        self.intermediate_size = intermediate_size
        self.max_position_embeddings = max_position_embeddings
        self.num_experts = num_experts
        self.top_k = top_k
        self.capacity_factor = capacity_factor
        self.router_aux_loss_coef = router_aux_loss_coef
        self.use_cache = False
        self.tie_word_embeddings = False


@dataclass
class MoECausalLMOutput(ModelOutput):
    loss: Optional[torch.FloatTensor] = None
    logits: Optional[torch.FloatTensor] = None
    router_aux_loss: Optional[torch.FloatTensor] = None
    expert_counts: Optional[torch.FloatTensor] = None


class MoEBlock(nn.Module):
    def __init__(self, config: MoEConfig):
        super().__init__()
        self.norm_attn = nn.LayerNorm(config.hidden_size)
        self.attn = nn.MultiheadAttention(
            config.hidden_size,
            config.num_attention_heads,
            dropout=0.0,
            batch_first=True,
        )
        self.norm_moe = nn.LayerNorm(config.hidden_size)
        self.router = nn.Linear(config.hidden_size, config.num_experts, bias=False)
        self.experts = nn.ModuleList(
            [
                nn.Sequential(
                    nn.Linear(config.hidden_size, config.intermediate_size),
                    nn.GELU(),
                    nn.Linear(config.intermediate_size, config.hidden_size),
                )
                for _ in range(config.num_experts)
            ]
        )
        self.num_experts = config.num_experts
        self.top_k = config.top_k

    def forward(self, hidden, causal_mask, key_padding_mask=None):
        residual = hidden
        normed = self.norm_attn(hidden)
        attended, _ = self.attn(
            normed,
            normed,
            normed,
            attn_mask=causal_mask,
            key_padding_mask=key_padding_mask,
            need_weights=False,
        )
        hidden = residual + attended

        residual = hidden
        normed = self.norm_moe(hidden)
        router_logits = self.router(normed)
        router_probs = torch.softmax(router_logits, dim=-1)
        top_values, top_indices = torch.topk(router_logits, self.top_k, dim=-1)
        gates = torch.softmax(top_values, dim=-1)

        flat = normed.reshape(-1, normed.shape[-1])
        flat_indices = top_indices.reshape(-1)
        flat_gates = gates.reshape(-1)
        token_indices = torch.arange(flat.shape[0], device=flat.device).unsqueeze(1)
        token_indices = token_indices.expand(-1, self.top_k).reshape(-1)
        dispatched = flat[token_indices]
        mixed = torch.zeros_like(flat)
        for expert_id, expert in enumerate(self.experts):
            selected = flat_indices == expert_id
            if selected.any():
                contribution = flat_gates[selected, None] * expert(dispatched[selected])
                mixed.index_add_(0, token_indices[selected], contribution)
        mixed = mixed.view_as(normed)

        assignment = torch.bincount(flat_indices, minlength=self.num_experts).to(normed.dtype)
        assignment = assignment / max(1, flat_indices.numel())
        importance = router_probs.mean(dim=(0, 1))
        aux_loss = self.num_experts * torch.sum(importance * assignment)
        return residual + mixed, aux_loss, assignment.detach()


class MoEForCausalLM(PreTrainedModel, GenerationMixin):
    config_class = MoEConfig
    base_model_prefix = "model"
    _no_split_modules = ["MoEBlock"]

    def __init__(self, config: MoEConfig):
        super().__init__(config)
        self.embed_tokens = nn.Embedding(config.vocab_size, config.hidden_size)
        self.embed_positions = nn.Embedding(config.max_position_embeddings, config.hidden_size)
        self.layers = nn.ModuleList([MoEBlock(config) for _ in range(config.num_hidden_layers)])
        self.norm = nn.LayerNorm(config.hidden_size)
        self.lm_head = nn.Linear(config.hidden_size, config.vocab_size, bias=False)
        self.post_init()
        self.last_router_aux_loss = None
        self.last_expert_counts = None

    def get_input_embeddings(self):
        return self.embed_tokens

    def set_input_embeddings(self, value):
        self.embed_tokens = value

    def _init_weights(self, module):
        if isinstance(module, (nn.Linear, nn.Embedding)):
            module.weight.data.normal_(mean=0.0, std=0.02)
            if isinstance(module, nn.Linear) and module.bias is not None:
                module.bias.data.zero_()
        elif isinstance(module, nn.LayerNorm):
            module.bias.data.zero_()
            module.weight.data.fill_(1.0)

    def forward(
        self,
        input_ids=None,
        attention_mask=None,
        labels=None,
        token_type_ids=None,
        position_ids=None,
        **kwargs,
    ):
        if input_ids is None:
            raise ValueError("input_ids é obrigatório")
        batch, sequence = input_ids.shape
        if sequence > self.config.max_position_embeddings:
            raise ValueError("A sequência excede max_position_embeddings.")
        positions = position_ids if position_ids is not None else torch.arange(sequence, device=input_ids.device).unsqueeze(0)
        hidden = self.embed_tokens(input_ids) + self.embed_positions(positions)
        causal_mask = torch.triu(
            torch.ones(sequence, sequence, device=input_ids.device, dtype=torch.bool), diagonal=1
        )
        padding = attention_mask.eq(0) if attention_mask is not None else None
        aux_losses = []
        counts = []
        for layer in self.layers:
            hidden, aux, layer_counts = layer(hidden, causal_mask, padding)
            aux_losses.append(aux)
            counts.append(layer_counts)
        hidden = self.norm(hidden)
        logits = self.lm_head(hidden)
        router_aux_loss = torch.stack(aux_losses).mean()
        expert_counts = torch.stack(counts).mean(dim=0)
        self.last_router_aux_loss = float(router_aux_loss.detach())
        self.last_expert_counts = expert_counts.detach().cpu().tolist()

        loss = None
        if labels is not None:
            shift_logits = logits[..., :-1, :].contiguous()
            shift_labels = labels[..., 1:].contiguous()
            loss = nn.functional.cross_entropy(
                shift_logits.view(-1, shift_logits.size(-1)),
                shift_labels.view(-1),
                ignore_index=-100,
            )
            loss = loss + self.config.router_aux_loss_coef * router_aux_loss
        return MoECausalLMOutput(
            loss=loss,
            logits=logits,
            router_aux_loss=router_aux_loss,
            expert_counts=expert_counts,
        )
