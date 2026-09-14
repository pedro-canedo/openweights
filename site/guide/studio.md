# Training Studio

The optional Studio adds local training to OpenWeights. It keeps its Python,
CUDA, OCR, checkpoints and SQLite data under the app data directory and does not
install Python or Docker globally.

## The five-click path

1. Open **Train** and choose **Install training module**. The first download is
   about 3 GB; scanned-page OCR is an optional download of about 66 MB.
2. Drop a PDF, text file, conversations, or a code folder into **Data**. One
   book is a valid input; native PDF text is processed page by page.
3. Choose automatic detection or correct the preset (**Book**, **Conversations**,
   or **Code**) and click **Prepare**. The Studio cleans layout, groups nearby
   duplicates, and creates separate train, validation and test partitions.
4. Review the summary and click **Train and generate model**. The default is
   Qwen3 0.6B, QLoRA, context 1,024, batch 1, accumulation 16, rank 16, up to
   50 steps and one pass over the available data.
5. When export finishes, click **Open in chat**. The GGUF and its manifest are
   published atomically in the same model library used by OpenWeights.

## Projects and base models

Each project keeps its sources, prepared dataset snapshots, runs and model
lineage together. Create a project for a subject, then add sources to it; the
same project can be reopened after a restart without uploading the files again.

The **Base model** picker starts with the pinned Qwen3 0.6B Apache-2.0 model.
Qwen3 1.7B is also available as a candidate and requires the local benchmark
before a run can start. The picker shows the expected download, VRAM, RAM and
disk requirements for the selected recipe. **Quick** is the safe 50-step
recipe; **Recommended** raises the budget only after the preflight succeeds.

You can import a compatible Hugging Face repository or a local model folder.
The Studio accepts dense safetensors models with a tokenizer and chat template;
quantized GGUF folders, adapter-only folders and models with custom code are
rejected with an actionable message. Imported models are copied into the
Studio data directory and pinned by revision and SHA-256, so changing the
original folder cannot change an existing run.

The **Advanced** panel exposes context, LoRA rank, learning rate and the time
limit. Changing one of these values recalculates the resource summary and
requires a new preflight. The chosen model revision and recipe are frozen in
the run manifest.

## Comparing results

After a run has completed, **Compare with base** runs the same short prompts
through the original and trained model, one at a time, and stores both answers
in the run record. The comparison is diagnostic only: it never uses the
reserved test partition to select a checkpoint or tune the recipe.

The module checks free VRAM, RAM and disk and runs a short benchmark before
training. If the chat engine is using the GPU, it asks before stopping it.
External GPU processes are detected through available memory and are never
terminated by the app.

## PDFs and OCR

Native text is never rejected because a PDF is long. OCR runs only on pages
without usable text. If the OCR component is not installed, click **Install
scanned-page reading** and then **Resume preparation**. Recognition is local and
includes Portuguese and English. Page progress and checkpoints prevent silent
truncation.

## Data safety and limits

Books and code use a causal objective; real conversations use supervised fine
tuning. The Studio does not invent questions and answers from prose. Small
inputs remain saved, but training is blocked when there is not enough material
for three usable partitions: “This content is too short to train and check the
result. Add more text.” The reserved test partition is not used to select a
checkpoint or tune parameters.

Cancel saves a safe checkpoint. Resume checks the dataset, base-model revision,
recipe and runtime before continuing. An incompatible checkpoint is reported
instead of silently changing the recipe.

## Troubleshooting

- **Access denied (Windows error 5):** close OpenWeights, wait a few seconds and
  retry installation. Antivirus software can briefly hold the runtime marker.
  Keep the app data folder writable and out of protected system directories.
- **Not enough GPU memory:** close other GPU consumers or choose 512-token
  context in **Advanced**. Capacity is validated on the real machine.
- **Training module unavailable:** downloads resume and catalog, signature and
  SHA-256 are verified before activation.

Removing the module deletes only its runtime and disposable caches. Datasets,
checkpoints and exported models remain by default.
