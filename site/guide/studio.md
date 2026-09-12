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
