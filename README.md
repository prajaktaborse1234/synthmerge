# synthmerge

**AI-powered conflict resolution for Git with your own workflow**

`synthmerge` is a minimalist, no-GUI command-line tool that leverages AI to resolve conflicts from `git cherry-pick` operations. It leverages the core principles and research of the [Patchpal project](https://gitlab.com/patchpal-ai), but focusing on a pure AI inference layer that works with your existing Git workflow. Reviews of the AI generated code still happen in your favorite editor.

---

## 🌟 Core Philosophy

1. **Separation of concerns**  
   Pure AI inference layer that doesn't duplicate Git functionality

2. **Git dependency**  
   Relies on Git's `diff3` conflict markers (requires `git config merge.conflictStyle diff3`)

3. **Developer workflow freedom**  
   Works with *any* editor (VS Code, Emacs, Vim, etc.) - no new GUI app required

4. **Model flexibility**  
   Removes the Patchpal fine-tuning requirement, allowing any instruct model to be used

5. **Parallel model inference with deduplication**  
   Queries an unlimited number of models in parallel and deduplicates their answers when they agree

---

## ✨ Key Features

- **Zero Git feature duplication**  
  Leverages Git's `diff3` conflict markers to identify context for patch application. If Git's conflict resolution isn't good enough, Git's conflict resolution should be improved rather than reinventing it.

- **Multi-AI endpoint support**  
  Supports parallel querying of any combination of models:
  - Patchpal-backend (fine-tuned specifically for patch resolution)
  - Self-hosted open source LLMs with OpenAI compatible endpoints
  - Gemini (using OpenAI-compatible API)

- **Review using your workflow**  
  Resolved conflicts appear in your editor with model attribution. All AI-generated code requires manual review before commit.

- **Fail-safe design**  
  If one model fails to resolve a conflict, Git's original conflict remains alongside solutions from other models for that hunk.

- **Configurable**  
  Configure inference servers, enable reasoning, set temperature, and other parameters.

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
   - You review in your editor (no GUI needed)

---

## ⚙️ Configuration

Create `~/.config/synthmerge.yaml` based on `synthmerge.yaml`:

```yaml
endpoints:

  - name: "Patchpal AI"
    type: "patchpal"
    url: "http://patchpal.usersys.redhat.com:9080/v1"

  - name: "llama.cpp vulkan"
    url: "http://localhost:8811/v1/chat/completions"
    type: "openai"
    model: "your favorite open weight open source coder model"
    api_key_file: "/dev/null"
    temperature: 0.7

  - name: "Gemini 2.5 pro"
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    type: "openai"
    model: "gemini-2.5-pro"
    api_key_file: "~/.gemini-api-key"
    reasoning_effort: "low"
```

> 💡 **Note**: `api_key_file: "/dev/null"` for local LLMs (like `llama.cpp`) is required by the API client but ignored.

---

## 🚀 Usage

```bash
# Ensure Git is configured for diff3
git config merge.conflictStyle diff3

# Attempt cherry-pick (will leave conflicts)
git cherry-pick -x <commit>

# Resolve conflicts with AI
synthmerge

# Review in your editor (no GUI needed!)
git diff
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
