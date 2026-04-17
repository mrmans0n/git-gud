# gg inbox

`gg inbox` muestra una vista de triage accionable a nivel de repositorio para todas las stacks locales. En vez de inspeccionar stack por stack, agrupa las PRs/MRs según qué necesitan ahora mismo.

Úsalo cuando quieras responder rápido a preguntas como:

- qué PRs están listas para land
- cuáles están bloqueadas por CI
- dónde han pedido cambios
- qué stacks se han quedado behind base

## Uso

```bash
gg inbox
gg inbox --all
gg inbox --json
```

## Buckets

`gg inbox` clasifica cada PR/MR en un único bucket, por prioridad:

1. `ready_to_land`
2. `changes_requested`
3. `blocked_on_ci`
4. `awaiting_review`
5. `behind_base`
6. `draft`
7. `merged` (solo con `--all`)

### Notas de clasificación

- Una PR con CI cancelado cuenta como `blocked_on_ci`.
- Si el refresco remoto falla de forma transitoria, la entrada no desaparece: sigue visible con un fallback razonable para que el inbox no quede vacío por un error temporal.
- `behind_base` se calcula comparando la tip real de la stack con `origin/<base>`, no el estado de tu rama base local.

## Ejemplo de salida humana

```text
Inbox (3 items across 2 stacks)

Ready to land (1):
  auth #2  abc1234  Add login button  stack/auth  PR #41

Blocked on CI (1):
  auth #3  def5678  Add login API  stack/auth  PR #42 ⏳

Awaiting review (1):
  billing #1  9876abc  Add invoice export  stack/billing  PR #51
```

## JSON

Con `--json`, `gg inbox` devuelve una respuesta versionada pensada para automatización y MCP.

Ejemplo:

```json
{
  "version": 1,
  "total_items": 2,
  "buckets": {
    "ready_to_land": [
      {
        "stack_name": "auth",
        "position": 1,
        "sha": "abc1234",
        "title": "Add login",
        "pr_number": 42,
        "pr_url": "https://github.com/org/repo/pull/42",
        "ci_status": "success",
        "behind_base": null
      }
    ],
    "blocked_on_ci": [
      {
        "stack_name": "auth",
        "position": 2,
        "sha": "def5678",
        "title": "Add login API",
        "pr_number": 43,
        "pr_url": "https://github.com/org/repo/pull/43",
        "ci_status": "running",
        "behind_base": 2
      }
    ]
  }
}
```

### Campos por entrada

- `stack_name`: nombre de la stack
- `position`: posición del commit en la stack
- `sha`: SHA corto
- `title`: título del commit
- `pr_number`: número de PR/MR
- `pr_url`: URL de la PR/MR
- `ci_status`: `pending`, `running`, `success`, `failed`, `canceled`, `unknown` o ausente
- `behind_base`: número de commits por detrás de `origin/<base>` o `null`

## Flags

- `--all`: incluye también elementos ya `merged`
- `--json`: emite salida estructurada para tooling/MCP

## Relación con otros comandos

- `gg ls` te enseña el estado detallado de la stack actual
- `gg log` te da una vista smartlog de la stack actual
- `gg inbox` sirve para triage transversal entre varias stacks
