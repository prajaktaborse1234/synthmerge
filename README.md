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

- **Model Flexibility**  
  No fine-tuning required, any instruct large language model can be used

- **Parallel Multi-AI Endpoint Support**  
  Simultaneously queries multiple AI models to resolve conflicts:
  - [Patchpal-backend](https://gitlab.com/patchpal-ai/patchpal-backend) (fine-tuned specifically for conflict resolution)
  - Self-hosted open-weight open source LLMs with OpenAI-compatible endpoints
  - Gemini (via OpenAI-compatible API)

- **Parameter Variants Support**  
  Each AI endpoint can be configured with multiple parameter variants to run multiple inference strategies:
  - Different reasoning effort levels (high, medium, low)
  - Temperature, top_p, top_k, min_p sampling parameters
  - Context handling options (no_context flag)

- **Results Deduplication**  
  Consolidates identical solutions and displays model and/or parameter variant agreement

- **Review Using Your Workflow**  
  - Resolved conflicts appear in your editor with model attribution
  - AI-generated code requires manual review before commit

- **Fail-Safe Design**  
  - When one model fails to resolve a conflict, Git's original conflict remains alongside solutions from other models for that hunk
  - Each AI endpoint can be configured with timeout, delay, and max_delay parameters

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

  - name: "Patchpal AI"
    type: "patchpal"
    url: "http://patchpal.usersys.redhat.com:9080/v1"
    #timeout: 600
    #retries: 10
    #delay: 1000
    #max_delay: 600000

  - name: "llama.cpp vulkan simple"
    url: "http://localhost:8811/v1/chat/completions"
    type: "openai"
    #no_context: false

  - name: "llama.cpp vulkan"
    url: "http://localhost:8811/v1/chat/completions"
    type: "openai"
    variants:
      # one query for each entry in the variants list
      - name: "default"
      - name: "min_p"
        temperature: 0.3
        top_p: 1.0
        top_k: 0
        min_p: 0.9
      - name: "no_context"
        no_context: true
    
  - name: "Gemini 2.5 pro"
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    type: "openai"
    model: "gemini-2.5-pro"
    api_key_file: "~/.gemini.api-key"
    variants:
      - reasoning_effort: "high"
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

> ✅ **Gemini supports a compatible OpenAI endpoint**  
> ✅ **Models work with stock weights** – the prompt engineering simulates Patchpal's fine-tuned behavior.

---

## 📊 Benchmark Statistics

The following statistics were generated using the `synthmerge_bench` tool on a C language dataset to evaluate model performance on conflict resolution tasks. These results may vary depending on prompt, context, and other variables. 

**Accuracy** checks if the AI resolved conflict is an exact match including all spaces, tabs, and newlines.

**Accuracy (aligned)** checks equality of whitespace patterns up until the first non-whitespace character, ignoring differences in lines without non-whitespace characters and whitespace variations after the first non-whitespace character (i.e. Python equivalence).

**Accuracy (stripped)** compresses all whitespaces and newlines into a single space (i.e. C/C++/Rust/JavaScript equivalence).

The probability that at least one of the three differnt PatchPal beams is exact (not ignoring  whitespace differences) is: 66.33% + 11.25% + 2.92% = 80.5%. This measurement used only new test data never exposed to the model during the fine tuning process.

```
# only the Beam 0 is comparable to the non Patchpal models
Model: Patchpal AI #0 (Beam search 0)
  Accuracy: 63.33% (715/1129)
  Accuracy (aligned): 67.76% (765/1129) # might be duplicate with other beams
  Accuracy (stripped): 71.12% (803/1129) # might be duplicate with other beams
  Error Rate: 0.53% (6/1129) # might be duplicate with other beams

# if Beam 0 is wrong, Beam 1 is right 11.25% of the time
Model: Patchpal AI #1 (Beam search 1) of the time
  Accuracy: 11.25% (127/1129)
  Accuracy (aligned): 22.50% (254/1129) # might be duplicate with other beams
  Accuracy (stripped): 33.92% (383/1129) # might be duplicate with other beams
  Error Rate: 0.53% (6/1129)

# if Beam 0 and Beam 1 are wrong, Beam 2 is right 2.92% of the time
Model: Patchpal AI #2 (Beam search 2)
  Accuracy: 2.92% (33/1129)
  Accuracy (aligned): 15.50% (175/1129) # might be duplicate with other beams
  Accuracy (stripped): 25.86% (292/1129) # might be duplicate with other beams
  Error Rate: 0.62% (7/1129)

Model: Gemini 2.5 pro (high) # reasoning_effort: high
  Accuracy: 55.18% (623/1129)
  Accuracy (aligned): 60.67% (685/1129)
  Accuracy (stripped): 63.42% (716/1129)
  Error Rate: 0.00% (0/1129)

Model: Gemini 2.5 pro (low) # reasoning_effort: low
  Accuracy: 51.64% (583/1129)
  Accuracy (aligned): 56.78% (641/1129)
  Accuracy (stripped): 58.90% (665/1129)
  Error Rate: 0.18% (2/1129)

# temperature: 0.7 top_k: 20 top_p: 0.8 min_p: 0
# llama.cpp vulkan Q6_K
Model: Qwen3-Coder-30B-A3B-Instruct (default)
  Accuracy: 48.54% (548/1129)
  Accuracy (aligned): 52.97% (598/1129)
  Accuracy (stripped): 55.89% (631/1129)
  Error Rate: 0.00% (0/1129)

# temperature: 0.7 top_k: 20 top_p: 0.8 min_p: 0
# llama.cpp vulkan Q6_K
Model: Qwen3-Coder-30B-A3B-Instruct (no_context)
  Accuracy: 45.88% (518/1129)
  Accuracy (aligned): 50.13% (566/1129)
  Accuracy (stripped): 53.59% (605/1129)
  Error Rate: 0.00% (0/1129)
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
