"""
NLLB-200-distilled-600M translation endpoint on Modal.

POST / — {"text": "...", "src_lang": "eng_Latn", "tgt_lang": "rus_Cyrl"}
        → {"text": "..."}

Runs on CPU (600M model fits comfortably, GPU not needed for short clauses).
buffer_containers=1 keeps one warm to avoid cold-start on first request.
"""

import modal

app = modal.App("foni-translate")

image = (
    modal.Image.debian_slim(python_version="3.11")
    .pip_install(
        "transformers==4.51.3",
        "torch==2.6.0",
        "sentencepiece==0.2.0",
        "fastapi[standard]==0.115.12",
        "protobuf==5.29.4",
    )
)

MODEL_ID = "facebook/nllb-200-distilled-600M"
DEFAULT_SRC = "eng_Latn"
DEFAULT_TGT = "rus_Cyrl"


@app.cls(
    image=image,
    gpu="T4",
    scaledown_window=300,
    buffer_containers=1,
)
class NLLBTranslator:
    @modal.enter()
    def load(self):
        from transformers import AutoModelForSeq2SeqLM, AutoTokenizer
        import torch

        print(f"[translate] loading {MODEL_ID}...")
        self.tokenizer = AutoTokenizer.from_pretrained(MODEL_ID)
        self.model = AutoModelForSeq2SeqLM.from_pretrained(MODEL_ID).to("cuda")
        self.model.eval()
        print("[translate] ready")

    @modal.fastapi_endpoint(method="POST")
    def translate(self, req: dict) -> dict:
        import torch
        import time

        text = req.get("text", "")
        src = req.get("src_lang", DEFAULT_SRC)
        tgt = req.get("tgt_lang", DEFAULT_TGT)

        if not text.strip():
            return {"text": ""}

        t0 = time.time()
        self.tokenizer.src_lang = src
        inputs = self.tokenizer(
            text,
            return_tensors="pt",
            padding=True,
            truncation=True,
            max_length=256,
        ).to("cuda")

        tgt_id = self.tokenizer.convert_tokens_to_ids(tgt)
        with torch.no_grad():
            out = self.model.generate(
                **inputs,
                forced_bos_token_id=tgt_id,
                max_length=256,
                num_beams=4,
            )

        result = self.tokenizer.decode(out[0], skip_special_tokens=True)
        elapsed = int((time.time() - t0) * 1000)
        print(f"[translate] {elapsed}ms | {src}→{tgt} | {len(text)}ch → {len(result)}ch")
        return {"text": result}


@app.local_entrypoint()
def test():
    translator = NLLBTranslator()
    cases = [
        "The deployment pipeline failed because the container image was not found in the registry.",
        "Let me push a fix to the main branch and redeploy.",
        "Hello, how are you?",
    ]
    for text in cases:
        result = translator.translate.remote({"text": text})
        print(f"IN:  {text}")
        print(f"OUT: {result['text']}")
        print()
