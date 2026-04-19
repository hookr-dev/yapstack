# Local LLM setup (llama.cpp, LM Studio, Ollama)

YapStack's AI chat can talk to any OpenAI-compatible local server. Point the custom provider at your server's base URL and you're done — no cloud account, no key, fully offline.

## llama.cpp (`llama-server`)

### Install

macOS: `brew install llama.cpp`. Other platforms: build from source at [github.com/ggerganov/llama.cpp](https://github.com/ggerganov/llama.cpp).

### Pick a model

For chat + YapStack's tool actions (rename note, save to notes, pin), use a model whose chat template supports tool calls:

- Qwen2.5-7B-Instruct
- Llama-3.1-8B-Instruct
- Mistral-Nemo-Instruct

Grab the GGUF from Hugging Face. A Q4_K_M quant is a good starting point on Apple Silicon.

### Start the server

```
llama-server -m /path/to/model.gguf -c 8192 --jinja --host 127.0.0.1 --port 8080
```

- `--jinja` is the critical flag — enables proper chat templating including tool calls. Without it, tool actions in YapStack won't fire.
- `-c 8192` gives you enough context for most transcripts. Raise for longer sessions if your model supports it.
- Metal acceleration is on by default on macOS.

### Configure YapStack

Settings → AI:

1. Provider: **Custom**
2. Base URL: `http://127.0.0.1:8080/v1`
3. API Key: leave blank
4. Click **Fetch Models from Server** → pick your model from the dropdown (or type it manually)
5. Click **Test Connection** — should say "Connected successfully"

## LM Studio

LM Studio exposes an OpenAI-compatible server at `http://localhost:1234/v1` by default. Load a model in the UI, start the server, then configure YapStack with that URL.

## Ollama

Ollama serves an OpenAI-compatible endpoint at `http://localhost:11434/v1`. `ollama pull llama3.1`, then point the custom provider at that URL. Model name is the Ollama tag (e.g., `llama3.1`).

## Troubleshooting

- **Test Connection fails with "fetch failed" or "ECONNREFUSED"** — server isn't running, wrong port, or bound to a different interface. Check the server is up and listening on the host/port you entered.
- **Chat works but tool actions never fire** — the model's chat template doesn't support tool calling, or you forgot `--jinja` on `llama-server`. Try a different model (see list above) or restart with `--jinja`.
- **Responses are very slow** — try a smaller model, a more aggressive quant (Q4_K_M → Q3_K_M), or make sure GPU acceleration is active (Metal on macOS, CUDA on Linux/Windows).
- **"HTTP 404" from Fetch Models** — your server doesn't expose `/v1/models`. You can still type the model name manually; this only disables the picker.
- **Ollama tool calls don't work** — Ollama's OpenAI-compat layer has limited tool-calling support depending on model. llama.cpp with `--jinja` is the most reliable.
