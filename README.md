# PRISM — PR Intelligent Stateful Manager

> Terminal UI for code review — manual, AI-assisted, or hybrid.

```
┌─ PRISM ─ SergioGutzB/prism ──────────────── [a] Agents  [S] Settings  [q] Quit ─┐
│  #  │ Título                              │ Autor      │ Edad   │ Labels          │
│ ────┼─────────────────────────────────────┼────────────┼────────┼──────────────── │
│ ▶ 8 │ feat: setup wizard with gh CLI      │ @sergio    │ 2h     │ [feature]       │
│   7 │ feat: wire real Jira ticket loading │ @sergio    │ 5h     │ [feature]       │
│   6 │ feat: wire review publish to GitHub │ @sergio    │ 1d     │ [feature]       │
└─────────────────────────────────────────────────────────────────────────────────┘
```

PRISM integra contexto de GitHub, sistemas de tickets (Jira, Linear) y agentes IA configurables en TOML para generar revisiones de código detalladas, con un flujo de **double-check humano** antes de publicar cualquier comentario.

---

## Características

- **Tres modos de review**: Solo IA, Solo manual, Híbrido
- **Agentes configurables en TOML** — 6 agentes built-in editables: Security, Architecture, Tests, Performance, Style, Summary
- **Integración con GitHub CLI** — detección automática de credenciales via `gh auth login`
- **Panel de tickets** — integración con Jira (Linear próximamente), carga no bloqueante con timeout de 5s
- **Double-check humano** — aprobar, rechazar o editar cada comentario antes de publicar
- **Navegación Vim** — `hjkl`, `gg`, `G`, `ctrl+d/u`, `dd`, `/` para buscar
- **Sin LLM → modo manual** — funciona sin API keys, solo GitHub es requerido

---

## Instalación

### Prerequisitos

- Rust 1.75+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- [GitHub CLI](https://cli.github.com/) (`gh`) — recomendado para autenticación automática

### Desde fuente

```bash
git clone https://github.com/SergioGutzB/prism.git
cd prism
cargo build --release
sudo mv target/release/prism /usr/local/bin/
```

---

## Configuración

### Autenticación automática (recomendado)

Si tienes `gh` instalado y autenticado, PRISM lo detecta automáticamente al iniciar:

```bash
gh auth login   # solo la primera vez
prism           # detecta token y repo automáticamente
```

Al confirmar, guarda las credenciales en `~/.config/prism/config.toml` para futuras sesiones.

### Variables de entorno

```bash
# Requerido (o usa gh auth login)
export GITHUB_TOKEN=ghp_...
export GITHUB_OWNER=tu_org
export GITHUB_REPO=tu_repo

# IA — alguna de estas (opcional — sin ellas solo modo manual)
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...
# O simplemente instala Claude Code CLI: npm install -g @anthropic-ai/claude-code

# Jira (opcional)
export JIRA_BASE_URL=https://empresa.atlassian.net
export JIRA_EMAIL=tu@email.com
export JIRA_API_TOKEN=tu_token
```

### Archivo de configuración

`~/.config/prism/config.toml` (creado automáticamente en el setup wizard):

```toml
[github]
token = "ghp_..."
owner = "mi_org"
repo  = "mi_repo"

[llm]
provider    = "anthropic"
model       = "claude-opus-4-5"
max_tokens  = 4096
temperature = 0.2

[agents]
agents_dir  = "~/.config/prism/agents"
concurrency = 3
timeout_secs = 60
```

---

## Uso

```bash
prism                                    # Usar config guardada
prism --owner empresa --repo mi-repo     # Repo específico (próximamente)
RUST_LOG=info prism 2>prism.log          # Con logging a archivo
```

---

## Flujo de trabajo

```
PrList → [Enter] → PrDetail
  ├── [r] Solo IA    → AgentRunner → DoubleCheck → SummaryPreview → Publish
  ├── [c] Manual     →              ReviewCompose → DoubleCheck → SummaryPreview → Publish
  └── [h] Híbrido    → AgentRunner → ReviewCompose → DoubleCheck → SummaryPreview → Publish
```

### Teclas globales

| Tecla | Acción |
|-------|--------|
| `j` / `k` | Navegar arriba/abajo |
| `gg` / `G` | Primer / último elemento |
| `ctrl+d` / `ctrl+u` | Media página |
| `/` | Buscar |
| `Tab` | Siguiente panel |
| `q` | Salir |
| `Esc` | Volver / cancelar |
| `a` | Configurar agentes |
| `S` | Settings |

### En PrDetail

| Tecla | Acción |
|-------|--------|
| `r` | Generar review con IA |
| `c` | Comentario manual |
| `h` | Modo híbrido |
| `f` | Ver árbol de archivos |
| `o` | Abrir en browser |

### En DoubleCheck

| Tecla | Acción |
|-------|--------|
| `Space` | Aprobar / rechazar comentario |
| `e` | Editar comentario |
| `A` / `D` | Aprobar todos / deseleccionar todos |
| `1-7` | Filtrar por agente |
| `p` | Ver preview del review |

---

## Agentes IA

Los agentes son archivos TOML en `~/.config/prism/agents/`. Los 6 built-in son editables:

| Agente | Descripción | Orden |
|--------|-------------|-------|
| 🔒 Security | OWASP Top 10, inyecciones, secretos expuestos | 1 |
| 🏛️ Architecture | SOLID, acoplamiento, patrones de diseño | 2 |
| 🧪 Tests | Cobertura, edge cases, mocks | 3 |
| ⚡ Performance | N+1 queries, complejidad, memory leaks | 4 |
| 📝 Style | Naming, legibilidad, código duplicado | 5 |
| 📋 Summary | Resumen ejecutivo para el cuerpo del review | 6 |

### Agente personalizado

```toml
# ~/.config/prism/agents/db_migrations.toml

[agent]
id          = "db_migrations"
name        = "Database Migrations"
description = "Revisa rollback, índices y locks en tablas grandes"
enabled     = true
order       = 10
icon        = "🗃️"
color       = "yellow"

[agent.prompt]
system = """
Eres un DBA experto. Revisa SOLO archivos de migración...
"""
prompt_suffix = """
Responde ÚNICAMENTE con JSON array. Si no hay migraciones, responde [].
Formato: {"file": "...", "line": null, "severity": "critical|warning|suggestion", "body": "..."}
"""

[agent.context]
include_diff         = true
include_pr_description = false
include_ticket       = false
include_file_list    = true
include_patterns     = ["migrations/*", "db/migrate/*"]
```

Un agente con el mismo `id` que un built-in lo sobreescribe completamente.

---

## Integración con IA

PRISM usa el **Claude Code CLI** (`claude`) si está instalado, sin necesidad de configurar API keys adicionales:

```bash
npm install -g @anthropic-ai/claude-code
claude login
prism  # los agentes IA funcionan automáticamente
```

También soporta `ANTHROPIC_API_KEY` y `OPENAI_API_KEY` directamente.

Si no hay IA configurada, el **modo manual siempre está disponible** — PRISM nunca falla por falta de LLM.

---

## Estructura del proyecto

```
prism/
├── src/
│   ├── main.rs              # Event loop principal (tokio)
│   ├── app.rs               # Estado global + máquina de estados
│   ├── config.rs            # Configuración (TOML + env vars + gh CLI)
│   ├── agents/              # Orquestador + runner (subprocess claude)
│   ├── github/              # Cliente GitHub REST API
│   ├── tickets/             # Providers: Jira, Linear (próximo)
│   ├── review/              # Modelos + publisher
│   ├── tui/                 # Terminal setup + event loop + keybindings
│   └── ui/                  # Pantallas + componentes (ratatui)
├── agents/                  # Agentes built-in en TOML
└── config/
    └── default.toml         # Configuración base
```

---

## Roadmap

- [x] Lista de PRs con filtro y búsqueda
- [x] Diff con syntax highlighting
- [x] Panel de ticket Jira (no bloqueante)
- [x] Review manual completo
- [x] Agentes IA en paralelo con progreso en tiempo real
- [x] Double-check: aprobar/rechazar/editar por comentario
- [x] Publicar review en GitHub
- [x] Setup wizard con detección automática de `gh auth`
- [ ] Provider Linear
- [ ] Persistencia de sesión
- [ ] Flag `--dry-run`
- [ ] Exportar review a Markdown
- [ ] Historial de reviews

---

## Licencia

MIT
