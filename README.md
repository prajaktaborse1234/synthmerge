# synthmerge

**AI-powered conflict resolution for Git**

`synthmerge` is a minimalistic command-line tool that leverages AI to automatically resolve conflicts arising from Git commands. Built on the research of the [Patchpal project](https://gitlab.com/patchpal-ai), it provides a pure AI inference layer that seamlessly integrates with your existing Git workflow. While the AI generates code solutions, all code reviews and approvals remain within your favorite code editor.

---

## 🎥 Demo

> ![synthmerge-demo](https://gitlab.com/aarcange/synthmerge-assets/-/raw/main/synthmerge-demo-0.1.8.webm)
> ![synthmerge-demo with ripgrep-edit](https://gitlab.com/aarcange/synthmerge-assets/-/raw/main/synthmerge-demo-0.1.8-ripgrep-edit.webm)
> ![synthmerge-demo with vim](https://gitlab.com/aarcange/synthmerge-assets/-/raw/main/synthmerge-demo-0.1.8-vim.webm)

---

## 🌟 Core Principles

1. **Specialized AI Layer**  
   Dedicated AI inference system that complements Git without duplicating its core functionality

2. **Git Integration**  
   Leverages Git's `diff3` conflict markers as the foundation (requires `git config merge.conflictStyle diff3`)

3. **Editor Agnostic**  
   Compatible with any development environment (VS Code, Emacs, Vim, etc.)

---

## ✨ Key Features

- **Universal Git Operation Support**  
  Seamlessly integrates with all Git operations that create conflicts:
  - `cherry-pick`
  - `merge`
  - `rebase`
  - `revert`
  - `stash pop`

- **Model Flexibility**  
  No fine-tuning required, any instruct large language model can be used

- **Parallel Multi-AI Endpoint Support**  
  Simultaneously queries multiple AI models to resolve conflicts:
  - [Patchpal-backend](https://gitlab.com/patchpal-ai/patchpal-backend) (fine-tuned specifically for conflict resolution)
  - Self-hosted open-weight open source LLMs with OpenAI-compatible endpoints
  - Gemini (via OpenAI-compatible API)
  - Claude (via Anthropic API)

- **Parameter Variants Support**  
  Each AI endpoint can be configured with multiple parameter variants to run multiple inference strategies:
  - Different reasoning effort levels (high, medium, low)
  - Temperature, top_p, top_k, min_p sampling parameters
  - Context handling options (context: no_diff: with_user_message: flags)
  - Custom JSON parameters that can be injected into the request payload from the YAML configuration (either at the endpoint level or in each variant)

- **Results Deduplication**  
  Consolidates identical solutions and displays model and/or parameter variant agreement

- **Review Using Your Workflow**  
  - Resolved conflicts appear in your editor with model attribution
  - AI-generated code requires manual review before commit

- **Fail-Safe Design**  
  - When one model fails to resolve a conflict, Git's original conflict remains alongside solutions from other models for that hunk
  - Each AI endpoint can be configured with timeout, delay, and max_delay parameters
  - Custom root certificates can be added to the endpoint configuration
  - Wait time between requests can be specified per endpoint

- **Benchmark**  
  Built-in benchmarking tool (`synthmerge_bench`) for evaluating model accuracy on conflict resolution tasks

---

## 🛠 How It Works

1. **Git sets up conflicts**  
   ```bash
   git config merge.conflictStyle diff3  # Must be set
   git cherry-pick -x <commit>           # Git detects conflicts
   ```

2. **synthmerge analyzes conflicts**  
   - Reads Git's `diff3` conflict markers
   - Extracts context (3 lines before/after conflict)
   - Generates precise AI prompt

3. **AI resolves conflict**  
   - Sends code + patch to configured endpoint
   - Receives resolved code

4. **Git gets updated**  
   - synthmerge inserts the AI resolution into existing diff3 markers
   - You review in your editor

> ✅ Works also for git rebase, revert and merge conflict resolutions.

---

## ⚙️ Configuration

Create `~/.config/synthmerge.yaml` based on `synthmerge.yaml`:

```yaml
endpoints:

  - name: "Claude Sonnet 4.5"
    url: "https://api.anthropic.com/v1/messages"
    type: "anthropic"
    x_api_key_file: "~/.keys/anthropic.api-key"
    json:
      max_tokens: 20000
      model: "claude-sonnet-4-5"
      temperature: 0
    headers:
      anthropic-version: "2023-06-01"
    variants:
      - name: "default"
      - name: "no_diff"
        context:
          no_diff: true
      #- name: "userctx"
      #  context:
      #    with_user_message: true

  - name: "Vertex Claude Sonnet 4.0"
    url: "https://host/path"
    type: "anthropic"
    api_key_file: "~/.keys/claude.api-key"
    json:
      anthropic_version: "something-YYYY-MM-DD"
      max_tokens: 20000
      temperature: 0
    variants:
      - name: "default"
      - name: "no_diff"
        context:
          no_diff: true
      #- name: "userctx"
      #  context:
      #    with_user_message: true
    # Optional root certificate for HTTPS endpoints
    # root_certificate_pem: "~/.ssl/corp-ca.pem"

  - name: "Patchpal AI"
    type: "patchpal"
    url: "http://patchpal.usersys.redhat.com:9080/v1"

  - name: "Gemini 3 pro preview"
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    type: "openai"
    api_key_file: "~/.gemini.api-key"
    json:
      model: "gemini-3-pro-preview"
      reasoning_effort: "low"

  - name: "Gemini 2.5 pro"
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    type: "openai"
    api_key_file: "~/.gemini.api-key"
    json:
      model: "gemini-2.5-pro"
      reasoning_effort: "low"

  - name: "llama.cpp vulkan minimal" # requires --no-jinja
    url: "http://localhost:8811/v1/chat/completions"
    type: "openai"

  - name: "llama.cpp vulkan" # requires --no-jinja
    url: "http://localhost:8811/v1/chat/completions"
    #timeout: 600000
    #retries: 10
    #delay: 1000
    #max_delay: 600000
    #wait: 1000
    type: "openai"
    #json:
    #  n_probs: 1
    variants:
      # one query for each entry in the variants list
      - name: "default"
      - name: "no_diff"
        context:
          no_diff: true
      #- name: "min_p"
      #  json:
      #    temperature: 0.3
      #    top_p: 1.0
      #    top_k: 0
      #    min_p: 0.9

  - name: "llama.cpp vulkan no_chat"
    url: "http://localhost:8811/v1/completions"
    type: "openai"
    no_chat: true
```

---

## 🚀 Usage

```bash
# Ensure Git is configured for diff3 conflict style
git config merge.conflictStyle diff3

# Attempt cherry-pick (will leave conflicts unresolved)
git cherry-pick -x <commit>

# Resolve conflicts with AI
synthmerge

# Review synthmerge resolved conflicts in each unmerged file ...
git diff --name-only --diff-filter=U

# ... or linearized in a single buffer to edit with ripgrep-edit
rg-edit -E vim -U -e '(?s)^<<<<<<<+ .*?^>>>>>>>+ '
rg-edit -E emacsclient -U -e '(?s)^<<<<<<<+ .*?^>>>>>>>+ '
```

---

## 🌐 Supported AI Endpoints

| Endpoint Type | Example Configuration | Notes |
|---------------|------------------------|-------|
| **Patchpal-backend** | `type: "patchpal"` | Fine-tuned for patch resolution |
| **OpenAI protocol** | `type: "openai"` | Self-hosted LLMs (e.g., `llama.cpp`) and Gemini |
| **Anthropic protocol** | `type: "anthropic"` | Claude models |

> ✅ **Gemini supports a compatible OpenAI endpoint**  
> ✅ **Models work with stock weights** – the prompt engineering simulates Patchpal's fine-tuned behavior.

---

## 📊 Benchmark Statistics

The following statistics were generated using the `synthmerge_bench` tool on a C language dataset to evaluate model performance on conflict resolution tasks. These results may vary depending on prompt, context, and other variables. 

**Accuracy** checks if the AI resolved conflict is an exact match including all spaces, tabs, and newlines.

**Accuracy (aligned)** checks equality of whitespace patterns up until the first non-whitespace character, ignoring differences in lines without non-whitespace characters and whitespace variations after the first non-whitespace character (i.e. Python equivalence).

**Accuracy (stripped)** compresses all whitespaces and newlines into a single space (i.e. C/C++/Rust/JavaScript equivalence).

This measurement used only new test data never exposed to the model during the fine tuning process.

```
Claude Sonnet 4.5 and Gemini 3 pro preview not done yet.

Model: Claude Sonnet 4.0 (default)
  Accuracy: 66.70% (753/1129)
  Accuracy (aligned): 70.42% (795/1129)
  Accuracy (stripped): 73.34% (828/1129)
  Error Rate: 0.00% (0/1129)
  Average tokens: 5730.47
  Average duration: 7.03 s

# only the Beam 0 is comparable to the non Patchpal models
Model: Patchpal AI #0
  Accuracy: 64.57% (729/1129)
  Accuracy (aligned): 68.47% (773/1129) # might be duplicate with other beams
  Accuracy (stripped): 71.12% (803/1129) # might be duplicate with other beams
  Error Rate: 0.44% (5/1129) # might be duplicate with other beams

Model: Claude Sonnet 4.0 (no_diff)
  Accuracy: 65.19% (736/1129)
  Accuracy (aligned): 68.29% (771/1129)
  Accuracy (stripped): 71.48% (807/1129)
  Error Rate: 0.00% (0/1129)
  Average tokens: 1184.14
  Average duration: 6.34 s

Model: Gemini 2.5 pro (high) # reasoning_effort: high
  Accuracy: 55.18% (623/1129)
  Accuracy (aligned): 60.67% (685/1129)
  Accuracy (stripped): 63.42% (716/1129)
  Error Rate: 0.00% (0/1129)

Model: Gemini 2.5 pro (low default)
  Accuracy: 53.06% (599/1129)
  Accuracy (aligned): 57.31% (647/1129)
  Accuracy (stripped): 59.96% (677/1129)
  Error Rate: 2.48% (28/1129)

Model: Gemini 2.5 pro (low no_diff)
  Accuracy: 49.16% (555/1129)
  Accuracy (aligned): 52.61% (594/1129)
  Accuracy (stripped): 54.38% (614/1129)
  Error Rate: 3.37% (38/1129)

# temperature: 0.7 top_k: 20 top_p: 0.8 min_p: 0
# llama.cpp vulkan Q6_K
Model: Qwen3-Coder-30B-A3B-Instruct (default)
  Accuracy: 48.72% (550/1129)
  Accuracy (aligned): 53.32% (602/1129)
  Accuracy (stripped): 56.07% (633/1129)
  Error Rate: 0.27% (3/1129)
  Average tokens: 4258.48
  Average duration: 9.85 s

# temperature: 0.7 top_k: 20 top_p: 0.8 min_p: 0
# llama.cpp vulkan Q6_K
Model: Qwen3-Coder-30B-A3B-Instruct (no_diff)
  Accuracy: 46.50% (525/1129)
  Accuracy (aligned): 50.84% (574/1129)
  Accuracy (stripped): 53.76% (607/1129)
  Error Rate: 0.09% (1/1129)
  Average tokens: 902.49
  Average duration: 4.46 s

# if Beam 0 is wrong, Beam 1 is right 10.54% of the time
Model: Patchpal AI #1
  Accuracy: 10.54% (119/1129)
  Accuracy (aligned): 21.17% (239/1129) # might be duplicate with other beams
  Accuracy (stripped): 30.03% (339/1129) # might be duplicate with other beams
  Error Rate: 0.53% (6/1129) # might be duplicate with other beams

Model: Qwen3-Coder-30B-A3B-Instruct (default) # perplexity beam #1
  Accuracy: 6.31% (71/1125)
  Accuracy (aligned): 10.40% (117/1125)
  Accuracy (stripped): 16.71% (188/1125)

# if Beam 0 and Beam 1 are wrong, Beam 2 is right 3.37% of the time
Model: Patchpal AI #2
  Accuracy: 3.37% (38/1129)
  Accuracy (aligned): 16.21% (183/1129) # might be duplicate with other beams
  Accuracy (stripped): 23.83% (269/1129) # might be duplicate with other beams
  Error Rate: 0.44% (5/1129) # might be duplicate with other beams

Model: Qwen3-Coder-30B-A3B-Instruct (default) # perplexity beam #2
  Accuracy: 1.72% (19/1103)
  Accuracy (aligned): 6.35% (70/1103)
  Accuracy (stripped): 10.79% (119/1103)
```

---

## 🛠 Installation

Build from source:

```bash
git clone https://github.com/aarcange/synthmerge.git
cd synthmerge
cargo build --release
```

---

## License

[![License: GPL-3.0-or-later](https://img.shields.io/badge/License-GPL--3.0--or--later-blue.svg)](https://www.gnu.org/licenses/gpl-3.0.html)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg)](https://www.gnu.org/licenses/agpl-3.0.html)
