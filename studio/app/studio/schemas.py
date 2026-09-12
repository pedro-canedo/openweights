from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator


class Recipe(BaseModel):
    model_config = ConfigDict(extra='forbid')
    name: str = Field(default='Meu dataset', min_length=1, max_length=100)
    source_ids: list[str] = Field(min_length=1, max_length=2000)
    parent_id: str | None = None
    preset: Literal['documents', 'code', 'conversations', 'mixed', 'quick', 'quality', 'text'] = 'documents'
    min_chars: int = Field(default=40, ge=1, le=10000)
    max_chars: int = Field(default=1000000, ge=100, le=4000000)
    chunk_chars: int = Field(default=4000, ge=128, le=64000)
    overlap: int = Field(default=0, ge=0, le=16000)
    chunking: Literal['sentence', 'paragraph', 'fixed', 'document'] = 'sentence'
    max_words: int = Field(default=180, ge=8, le=2000)
    min_words: int = Field(default=12, ge=1, le=500)
    overlap_words: int = Field(default=0, ge=0, le=500)
    quality_filter: Literal['off', 'light', 'strict'] = 'light'
    drop_boilerplate: bool = False
    min_quality: float = Field(default=0, ge=0, le=1)
    llm_classify: bool = False
    llm_model: str = Field(default='openai/gpt-4o-mini', min_length=1, max_length=200)
    llm_base_url: str = Field(default='https://openrouter.ai/api/v1', min_length=8, max_length=300)
    separator: str = Field(default='\n\n', min_length=1, max_length=100)
    exact_dedupe: bool = True
    near_dedupe: bool = False
    similarity: float = Field(default=0.9, ge=0.5, le=1)
    clean_html: bool = True
    boilerplate: bool = True
    normalize_spaces: bool = True
    normalize_markdown: bool = False
    min_example_chars: int = Field(default=1, ge=1, le=64000)
    tokenizer_model_id: str | None = None
    max_tokens: int = Field(default=1024, ge=128, le=8192)
    long_examples: Literal['reject', 'skip', 'truncate_text'] = 'reject'
    metadata: bool = True
    detect_language: bool = True
    language: Literal['any', 'pt', 'en', 'es'] = 'any'
    include_docs: bool = True
    include_tests: bool = True
    include_config: bool = True
    include_comments: bool = True
    tables: bool = True
    extensions: list[str] = Field(default_factory=list, max_length=100)
    exclude: list[str] = Field(default_factory=list, max_length=100)
    validation: float = Field(default=0.1, ge=0.01, le=0.4)
    test: float = Field(default=0.1, ge=0, le=0.4)
    grouping: Literal['document', 'source', 'sequence'] = 'document'
    seed: int = Field(default=42, ge=0, le=2147483647)
    invalid: Literal['skip', 'fail'] = 'skip'
    instruction_field: str = Field(default='instruction', max_length=100)
    input_field: str = Field(default='input', max_length=100)
    output_field: str = Field(default='output', max_length=100)
    ocr: bool = False
    ocr_language: Literal['por', 'eng', 'spa', 'por+eng'] = 'por+eng'
    max_pdf_pages: int = Field(default=10000, ge=1, le=20000)
    max_files: int = Field(default=2000, ge=1, le=10000)
    max_depth: int = Field(default=12, ge=1, le=30)
    expanded_mb: int = Field(default=256, ge=1, le=1024)
    workers: int = Field(default=2, ge=1, le=4)
    memory_mb: int = Field(default=4096, ge=1024, le=16384)
    timeout_seconds: int = Field(default=900, ge=30, le=3600)

    @model_validator(mode='after')
    def bounds(self):
        self.name = self.name.strip()
        if not self.name:
            raise ValueError('Informe um nome para o dataset.')
        self.extensions = [ext.lower() if ext.startswith('.') else '.'+ext.lower() for ext in self.extensions]
        self.llm_model = self.llm_model.strip()
        self.llm_base_url = self.llm_base_url.strip().rstrip('/')
        if self.overlap >= self.chunk_chars or self.min_chars > self.max_chars:
            raise ValueError('Overlap deve ser menor que o bloco; mínimo deve ser menor que máximo.')
        if self.overlap_words >= self.max_words or self.min_words > self.max_words:
            raise ValueError('Sobreposição e mínimo de palavras devem ser menores que o máximo de palavras.')
        if not self.llm_base_url.startswith('https://') or ' ' in self.llm_base_url:
            raise ValueError('A URL do classificador deve ser HTTPS, sem espaços.')
        if len(set(self.source_ids)) != len(self.source_ids):
            raise ValueError('Fontes repetidas na receita.')
        return self


class RemoteSource(BaseModel):
    model_config = ConfigDict(extra='forbid')
    urls: list[str] = Field(min_length=1, max_length=20)
    repository: bool = False
    revision: str = Field(default='main', min_length=1, max_length=100)
    license: str = Field(default='', max_length=200)


class Publish(BaseModel):
    model_config = ConfigDict(extra='forbid')
    excluded_ids: list[str] = Field(default_factory=list, max_length=100000)
