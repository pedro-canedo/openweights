import torch

from app.moe import MoEConfig, MoEForCausalLM


def test_moe_forward_returns_loss_and_routing_stats():
    config = MoEConfig(
        vocab_size=32,
        hidden_size=32,
        num_hidden_layers=2,
        num_attention_heads=4,
        intermediate_size=64,
        max_position_embeddings=16,
        num_experts=4,
        top_k=2,
    )
    model = MoEForCausalLM(config)
    input_ids = torch.randint(0, config.vocab_size, (2, 8))
    output = model(input_ids=input_ids, labels=input_ids)
    assert output.logits.shape == (2, 8, config.vocab_size)
    assert torch.isfinite(output.loss)
    assert torch.isfinite(output.router_aux_loss)
    assert len(output.expert_counts) == config.num_experts
    assert abs(float(output.expert_counts.sum()) - 1.0) < 1e-5
