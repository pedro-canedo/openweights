from typing import Literal
from pydantic import BaseModel, Field, field_validator, model_validator


class ModelInput(BaseModel):
    name: str = Field(min_length=1, max_length=100)
    source: Literal['hub', 'local'] = 'hub'
    repo: str = Field(min_length=1, max_length=200)
    revision: str = Field(default='main', min_length=1, max_length=100)


class TrainInput(BaseModel):
    name: str = Field(min_length=1, max_length=100)
    mode: Literal['qlora', 'continued', 'scratch', 'scratch_moe'] = 'qlora'
    model_id: str | None = None
    dataset_id: str
    context: int = Field(default=1024, ge=64, le=8192)
    batch: int = Field(default=1, ge=1, le=16)
    accumulation: int = Field(default=16, ge=1, le=128)
    max_steps: int = Field(default=100, ge=1, le=1000000)
    learning_rate: float = Field(default=1e-4, ge=1e-7, le=0.01)
    rank: int = Field(default=16, ge=4, le=128)
    validation: float = Field(default=0.1, ge=0.05, le=0.4)
    seed: int = Field(default=42, ge=0, le=2147483647)
    save_steps: int = Field(default=50, ge=1, le=10000)
    gradient_checkpointing: bool = True
    text_packing: Literal['pack', 'document'] = 'pack'
    eos_policy: Literal['append', 'none'] = 'append'
    scratch_size: Literal['tiny', 'small'] = 'small'
    num_experts: int = Field(default=4, ge=2, le=16)
    top_k: int = Field(default=2, ge=1, le=4)
    capacity_factor: float = Field(default=1.25, ge=1.0, le=2.0)
    router_aux_loss_coef: float = Field(default=0.01, ge=0, le=1)
    @model_validator(mode='after')
    def valid_model(self):
        if self.top_k > self.num_experts:
            raise ValueError('top_k não pode ser maior que o número de experts.')
        if self.mode not in ('scratch', 'scratch_moe') and not self.model_id:
            raise ValueError('Escolha um modelo pré-treinado.')
        return self


class GenerateInput(BaseModel):
    model_id: str | None = None
    run_id: str | None = None
    prompt: str = Field(min_length=1, max_length=16000)
    system: str = Field(default='Responda em português de forma clara.', max_length=2000)
    max_tokens: int = Field(default=200, ge=1, le=1024)
    temperature: float = Field(default=0, ge=0, le=2)
    @model_validator(mode='after')
    def one_target(self):
        if bool(self.model_id) == bool(self.run_id):
            raise ValueError('Escolha um modelo base ou um treinamento concluído.')
        return self


class GgufExportInput(BaseModel):
    quantization: Literal['f16', 'bf16', 'q8_0', 'q5_k_m', 'q4_k_m'] = 'q4_k_m'

    @field_validator('quantization', mode='before')
    @classmethod
    def normalize_quantization(cls, value):
        return value.lower() if isinstance(value, str) else value
